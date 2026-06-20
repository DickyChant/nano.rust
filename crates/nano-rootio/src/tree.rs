use std::fmt::Debug;
use std::ops::Range;

use crate::decompress::decompress_root_blocks;
use crate::error::{Error, Result};
use crate::parse::{
    maybe_raw_buffer, parse_tnamed, read_raw_optional, read_tobjarray, skip_tiofeatures, Cursor,
    ObjectContext, RawObject,
};
use crate::root_file::{parse_tkey_header, Source};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafInfo {
    pub name: String,
    pub root_class: String,
    pub type_name: String,
    pub len: i32,
    pub element_size: i32,
    pub is_unsigned: bool,
    pub has_leaf_count: bool,
    pub leaf_count_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchInfo {
    pub name: String,
    pub types: Vec<String>,
    pub basket_count: usize,
    pub basket_first_entries: Vec<i64>,
}

#[derive(Debug, Clone)]
enum BasketStorage {
    InMemory(Vec<u8>),
    OnDisk { source: Source, seek: u64, len: u64 },
}

#[derive(Debug, Clone)]
struct Branch {
    name: String,
    entries: i64,
    branches: Vec<Branch>,
    leaves: Vec<LeafInfo>,
    basket_entry: Vec<i64>,
    baskets: Vec<BasketStorage>,
}

#[derive(Debug, Clone)]
pub struct Tree {
    name: String,
    entries: i64,
    branches: Vec<Branch>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ColumnRequest {
    ScalarF32(String),
    ScalarF64(String),
    ScalarI32(String),
    ScalarI64(String),
    ScalarU32(String),
    ScalarU64(String),
    Bool(String),
    JaggedF32 { branch: String, counter: String },
    JaggedF64 { branch: String, counter: String },
    JaggedI32 { branch: String, counter: String },
    JaggedI64 { branch: String, counter: String },
    JaggedU32 { branch: String, counter: String },
    JaggedU64 { branch: String, counter: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ColumnData {
    F32(Vec<f32>),
    F64(Vec<f64>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    U32(Vec<u32>),
    U64(Vec<u64>),
    Bool(Vec<bool>),
    JaggedF32(Vec<Vec<f32>>),
    JaggedF64(Vec<Vec<f64>>),
    JaggedI32(Vec<Vec<i32>>),
    JaggedI64(Vec<Vec<i64>>),
    JaggedU32(Vec<Vec<u32>>),
    JaggedU64(Vec<Vec<u64>>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnChunk {
    pub name: String,
    pub data: ColumnData,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeChunk {
    pub start: i64,
    pub len: usize,
    pub columns: Vec<ColumnChunk>,
}

pub struct ChunkedReader<'a> {
    tree: &'a Tree,
    requests: Vec<ColumnRequest>,
    next: i64,
    end: i64,
    chunk_size: usize,
}

pub trait Scalar: Sized + Copy + Debug + PartialEq + 'static {
    const TYPE_NAME: &'static str;
    const WIDTH: usize;
    fn decode(bytes: &[u8]) -> Self;
}

macro_rules! scalar_be {
    ($ty:ty, $name:literal, $width:literal, $from:ident) => {
        impl Scalar for $ty {
            const TYPE_NAME: &'static str = $name;
            const WIDTH: usize = $width;
            fn decode(bytes: &[u8]) -> Self {
                <$ty>::$from(bytes.try_into().unwrap())
            }
        }
    };
}

impl Scalar for u8 {
    const TYPE_NAME: &'static str = "u8";
    const WIDTH: usize = 1;
    fn decode(bytes: &[u8]) -> Self {
        bytes[0]
    }
}

impl Scalar for i8 {
    const TYPE_NAME: &'static str = "i8";
    const WIDTH: usize = 1;
    fn decode(bytes: &[u8]) -> Self {
        bytes[0] as i8
    }
}

scalar_be!(u16, "u16", 2, from_be_bytes);
scalar_be!(i16, "i16", 2, from_be_bytes);
scalar_be!(u32, "u32", 4, from_be_bytes);
scalar_be!(i32, "i32", 4, from_be_bytes);
scalar_be!(u64, "u64", 8, from_be_bytes);
scalar_be!(i64, "i64", 8, from_be_bytes);

impl Scalar for f32 {
    const TYPE_NAME: &'static str = "f32";
    const WIDTH: usize = 4;
    fn decode(bytes: &[u8]) -> Self {
        f32::from_bits(u32::from_be_bytes(bytes.try_into().unwrap()))
    }
}

impl Scalar for f64 {
    const TYPE_NAME: &'static str = "f64";
    const WIDTH: usize = 8;
    fn decode(bytes: &[u8]) -> Self {
        f64::from_bits(u64::from_be_bytes(bytes.try_into().unwrap()))
    }
}

impl Scalar for bool {
    const TYPE_NAME: &'static str = "bool";
    const WIDTH: usize = 1;
    fn decode(bytes: &[u8]) -> Self {
        bytes[0] != 0
    }
}

impl Tree {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn entries(&self) -> i64 {
        self.entries
    }

    pub fn branches(&self) -> Vec<BranchInfo> {
        self.branch_refs()
            .into_iter()
            .map(|branch| BranchInfo {
                name: branch.name.clone(),
                types: branch
                    .leaves
                    .iter()
                    .map(|leaf| leaf.type_name.clone())
                    .collect(),
                basket_count: branch.baskets.len(),
                basket_first_entries: branch.basket_entry.clone(),
            })
            .collect()
    }

    pub fn read_scalar<T: Scalar>(&self, branch_name: &str) -> Result<Vec<T>> {
        let branch = self.find_branch(branch_name)?;
        self.read_scalar_range(branch_name, 0, branch.entries.max(0) as usize)
    }

    pub fn read_scalar_range<T: Scalar>(
        &self,
        branch_name: &str,
        start: i64,
        len: usize,
    ) -> Result<Vec<T>> {
        let branch = self.find_branch(branch_name)?;
        let leaf = branch.scalar_leaf()?;
        if leaf.type_name != T::TYPE_NAME {
            return Err(Error::TypeMismatch {
                branch: branch_name.to_string(),
                root_type: leaf.type_name.clone(),
                requested: T::TYPE_NAME,
            });
        }
        let range = checked_window(branch.entries, start, len)?;
        let mut out = Vec::with_capacity(range_len(&range));
        for basket_index in branch.overlapping_basket_indices(&range) {
            let basket_start = branch.basket_start(basket_index)?;
            let basket_end = branch.basket_end(basket_index)?;
            let overlap_start = range.start.max(basket_start);
            let overlap_end = range.end.min(basket_end);
            let local_start = usize::try_from(overlap_start - basket_start).unwrap();
            let local_len = usize::try_from(overlap_end - overlap_start).unwrap();
            let basket = &branch.baskets[basket_index];
            let (entries, payload) = read_basket_payload(basket)?;
            let need = entries as usize * T::WIDTH;
            if payload.len() < need {
                return Err(Error::parse(
                    payload.len(),
                    format!("basket for {branch_name} needs {need} bytes"),
                ));
            }
            let byte_start = local_start
                .checked_mul(T::WIDTH)
                .ok_or_else(|| Error::parse(0, "scalar basket byte offset overflow"))?;
            let byte_len = local_len
                .checked_mul(T::WIDTH)
                .ok_or_else(|| Error::parse(0, "scalar basket byte length overflow"))?;
            let byte_end = byte_start
                .checked_add(byte_len)
                .ok_or_else(|| Error::parse(0, "scalar basket byte range overflow"))?;
            if byte_end > payload.len() {
                return Err(Error::parse(
                    payload.len(),
                    format!("basket for {branch_name} lacks requested byte range"),
                ));
            }
            for chunk in payload[byte_start..byte_end].chunks_exact(T::WIDTH) {
                out.push(T::decode(chunk));
            }
        }
        Ok(out)
    }

    pub fn read_jagged<T: Scalar>(
        &self,
        branch_name: &str,
        counter_branch_name: &str,
    ) -> Result<Vec<Vec<T>>> {
        let branch = self.find_branch(branch_name)?;
        self.read_jagged_range(
            branch_name,
            counter_branch_name,
            0,
            branch.entries.max(0) as usize,
        )
    }

    pub fn read_jagged_auto<T: Scalar>(&self, branch_name: &str) -> Result<Vec<Vec<T>>> {
        let counter = self
            .find_branch(branch_name)?
            .jagged_leaf::<T>()?
            .leaf_count_name
            .clone()
            .ok_or_else(|| Error::unsupported(branch_name, "jagged leaf has no counter name"))?;
        self.read_jagged(branch_name, &counter)
    }

    pub fn read_jagged_range<T: Scalar>(
        &self,
        branch_name: &str,
        counter_branch_name: &str,
        start: i64,
        len: usize,
    ) -> Result<Vec<Vec<T>>> {
        let branch = self.find_branch(branch_name)?;
        let _leaf = branch.jagged_leaf::<T>()?;
        let counter = self.find_branch(counter_branch_name)?;
        let counter_leaf = counter.scalar_leaf()?;
        if counter_leaf.type_name != u32::TYPE_NAME {
            return Err(Error::TypeMismatch {
                branch: counter_branch_name.to_string(),
                root_type: counter_leaf.type_name.clone(),
                requested: u32::TYPE_NAME,
            });
        }
        if branch.entries != counter.entries {
            return Err(Error::unsupported(
                branch_name,
                format!(
                    "jagged branch has {} entries but counter {counter_branch_name} has {}",
                    branch.entries, counter.entries
                ),
            ));
        }

        let range = checked_window(branch.entries, start, len)?;
        let mut out = Vec::with_capacity(range_len(&range));
        for basket_index in branch.overlapping_basket_indices(&range) {
            let basket_start = branch.basket_start(basket_index)?;
            let basket_end = branch.basket_end(basket_index)?;
            let overlap_start = range.start.max(basket_start);
            let overlap_end = range.end.min(basket_end);
            let counts_len = usize::try_from(overlap_end - basket_start).unwrap();
            let counts =
                self.read_scalar_range::<u32>(counter_branch_name, basket_start, counts_len)?;
            let local_start = usize::try_from(overlap_start - basket_start).unwrap();
            let overlap_len = usize::try_from(overlap_end - overlap_start).unwrap();
            let prefix_elems = sum_counts(branch_name, &counts[..local_start])?;
            let requested_counts = &counts[local_start..local_start + overlap_len];
            let requested_elems = sum_counts(branch_name, requested_counts)?;

            let (_, payload) = read_basket_payload(&branch.baskets[basket_index])?;
            let byte_start = prefix_elems
                .checked_mul(T::WIDTH)
                .ok_or_else(|| Error::parse(0, "jagged basket byte offset overflow"))?;
            let byte_len = requested_elems
                .checked_mul(T::WIDTH)
                .ok_or_else(|| Error::parse(0, "jagged basket byte length overflow"))?;
            let byte_end = byte_start
                .checked_add(byte_len)
                .ok_or_else(|| Error::parse(0, "jagged basket byte range overflow"))?;
            if byte_end > payload.len() {
                return Err(Error::parse(
                    payload.len(),
                    format!("basket for {branch_name} lacks requested jagged byte range"),
                ));
            }
            let mut pos = byte_start;
            for &count in requested_counts {
                let row_len = usize::try_from(count)
                    .map_err(|_| Error::parse(0, "jagged counter overflows usize"))?;
                let mut row = Vec::with_capacity(row_len);
                for _ in 0..row_len {
                    let next = pos + T::WIDTH;
                    row.push(T::decode(&payload[pos..next]));
                    pos = next;
                }
                out.push(row);
            }
        }
        Ok(out)
    }

    pub fn read_jagged_range_auto<T: Scalar>(
        &self,
        branch_name: &str,
        start: i64,
        len: usize,
    ) -> Result<Vec<Vec<T>>> {
        let counter = self
            .find_branch(branch_name)?
            .jagged_leaf::<T>()?
            .leaf_count_name
            .clone()
            .ok_or_else(|| Error::unsupported(branch_name, "jagged leaf has no counter name"))?;
        self.read_jagged_range(branch_name, &counter, start, len)
    }

    pub fn read_chunk(
        &self,
        start: i64,
        len: usize,
        requests: &[ColumnRequest],
    ) -> Result<TreeChunk> {
        let range = checked_window(self.entries, start, len)?;
        let len = range_len(&range);
        let mut columns = Vec::with_capacity(requests.len());
        for request in requests {
            columns.push(self.read_column_chunk(range.start, len, request)?);
        }
        Ok(TreeChunk {
            start: range.start,
            len,
            columns,
        })
    }

    pub fn chunked_reader(
        &self,
        start: i64,
        len: usize,
        chunk_size: usize,
        requests: Vec<ColumnRequest>,
    ) -> Result<ChunkedReader<'_>> {
        if chunk_size == 0 {
            return Err(Error::unsupported(
                "chunked reader",
                "chunk size must be positive",
            ));
        }
        let range = checked_window(self.entries, start, len)?;
        Ok(ChunkedReader {
            tree: self,
            requests,
            next: range.start,
            end: range.end,
            chunk_size,
        })
    }

    fn read_column_chunk(
        &self,
        start: i64,
        len: usize,
        request: &ColumnRequest,
    ) -> Result<ColumnChunk> {
        let (name, data) = match request {
            ColumnRequest::ScalarF32(name) => (
                name.clone(),
                ColumnData::F32(self.read_scalar_range(name, start, len)?),
            ),
            ColumnRequest::ScalarF64(name) => (
                name.clone(),
                ColumnData::F64(self.read_scalar_range(name, start, len)?),
            ),
            ColumnRequest::ScalarI32(name) => (
                name.clone(),
                ColumnData::I32(self.read_scalar_range(name, start, len)?),
            ),
            ColumnRequest::ScalarI64(name) => (
                name.clone(),
                ColumnData::I64(self.read_scalar_range(name, start, len)?),
            ),
            ColumnRequest::ScalarU32(name) => (
                name.clone(),
                ColumnData::U32(self.read_scalar_range(name, start, len)?),
            ),
            ColumnRequest::ScalarU64(name) => (
                name.clone(),
                ColumnData::U64(self.read_scalar_range(name, start, len)?),
            ),
            ColumnRequest::Bool(name) => (
                name.clone(),
                ColumnData::Bool(self.read_scalar_range(name, start, len)?),
            ),
            ColumnRequest::JaggedF32 { branch, counter } => (
                branch.clone(),
                ColumnData::JaggedF32(self.read_jagged_range(branch, counter, start, len)?),
            ),
            ColumnRequest::JaggedF64 { branch, counter } => (
                branch.clone(),
                ColumnData::JaggedF64(self.read_jagged_range(branch, counter, start, len)?),
            ),
            ColumnRequest::JaggedI32 { branch, counter } => (
                branch.clone(),
                ColumnData::JaggedI32(self.read_jagged_range(branch, counter, start, len)?),
            ),
            ColumnRequest::JaggedI64 { branch, counter } => (
                branch.clone(),
                ColumnData::JaggedI64(self.read_jagged_range(branch, counter, start, len)?),
            ),
            ColumnRequest::JaggedU32 { branch, counter } => (
                branch.clone(),
                ColumnData::JaggedU32(self.read_jagged_range(branch, counter, start, len)?),
            ),
            ColumnRequest::JaggedU64 { branch, counter } => (
                branch.clone(),
                ColumnData::JaggedU64(self.read_jagged_range(branch, counter, start, len)?),
            ),
        };
        Ok(ColumnChunk { name, data })
    }

    fn find_branch(&self, branch_name: &str) -> Result<&Branch> {
        self.branch_refs()
            .into_iter()
            .find(|branch| branch.name == branch_name)
            .ok_or_else(|| Error::MissingBranch(branch_name.to_string()))
    }

    fn branch_refs(&self) -> Vec<&Branch> {
        let mut out = Vec::new();
        for branch in &self.branches {
            branch.collect_refs(&mut out);
        }
        out
    }
}

impl Iterator for ChunkedReader<'_> {
    type Item = Result<TreeChunk>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.end {
            return None;
        }
        let remaining = usize::try_from(self.end - self.next).ok()?;
        let len = remaining.min(self.chunk_size);
        let start = self.next;
        self.next += len as i64;
        Some(self.tree.read_chunk(start, len, &self.requests))
    }
}

impl Branch {
    fn collect_refs<'a>(&'a self, out: &mut Vec<&'a Branch>) {
        if self.branches.is_empty() {
            out.push(self);
        } else {
            out.push(self);
            for branch in &self.branches {
                branch.collect_refs(out);
            }
        }
    }

    fn scalar_leaf(&self) -> Result<&LeafInfo> {
        let [leaf] = self.leaves.as_slice() else {
            return Err(Error::unsupported(
                &self.name,
                format!("expected exactly one leaf, found {}", self.leaves.len()),
            ));
        };
        if leaf.len != 1 {
            return Err(Error::unsupported(
                &self.name,
                format!("fixed arrays are not P1 scalar branches (len={})", leaf.len),
            ));
        }
        if leaf.has_leaf_count {
            return Err(Error::unsupported(
                &self.name,
                "leaf-count branch must be read with read_jagged",
            ));
        }
        Ok(leaf)
    }

    fn jagged_leaf<T: Scalar>(&self) -> Result<&LeafInfo> {
        let [leaf] = self.leaves.as_slice() else {
            return Err(Error::unsupported(
                &self.name,
                format!("expected exactly one leaf, found {}", self.leaves.len()),
            ));
        };
        if leaf.type_name != T::TYPE_NAME {
            return Err(Error::TypeMismatch {
                branch: self.name.clone(),
                root_type: leaf.type_name.clone(),
                requested: T::TYPE_NAME,
            });
        }
        if !leaf.has_leaf_count {
            return Err(Error::unsupported(
                &self.name,
                "branch has no leaf-count metadata",
            ));
        }
        Ok(leaf)
    }

    fn basket_start(&self, index: usize) -> Result<i64> {
        self.basket_entry
            .get(index)
            .copied()
            .ok_or_else(|| Error::parse(0, format!("missing basket {index} first-entry metadata")))
    }

    fn basket_end(&self, index: usize) -> Result<i64> {
        if index + 1 < self.basket_entry.len() {
            Ok(self.basket_entry[index + 1])
        } else {
            Ok(self.entries)
        }
    }

    fn overlapping_basket_indices(&self, range: &Range<i64>) -> Vec<usize> {
        if range.start == range.end {
            return Vec::new();
        }
        (0..self.baskets.len().min(self.basket_entry.len()))
            .filter(|&index| {
                let start = self.basket_entry[index];
                let end = if index + 1 < self.basket_entry.len() {
                    self.basket_entry[index + 1]
                } else {
                    self.entries
                };
                start < range.end && end > range.start
            })
            .collect()
    }
}

fn checked_window(entries: i64, start: i64, len: usize) -> Result<Range<i64>> {
    if start < 0 {
        return Err(Error::parse(0, format!("negative entry start {start}")));
    }
    let len_i64 = i64::try_from(len)
        .map_err(|_| Error::parse(0, format!("entry window length {len} overflows i64")))?;
    let end = start
        .checked_add(len_i64)
        .ok_or_else(|| Error::parse(0, "entry window end overflows i64"))?;
    if end > entries {
        return Err(Error::parse(
            0,
            format!("entry window [{start}, {end}) exceeds branch entries {entries}"),
        ));
    }
    Ok(start..end)
}

fn range_len(range: &Range<i64>) -> usize {
    usize::try_from(range.end - range.start).unwrap_or(0)
}

fn sum_counts(branch_name: &str, counts: &[u32]) -> Result<usize> {
    counts.iter().try_fold(0_usize, |acc, &count| {
        acc.checked_add(count as usize).ok_or_else(|| {
            Error::parse(
                0,
                format!("jagged element count overflow while reading {branch_name}"),
            )
        })
    })
}

pub(crate) fn parse_tree<'a>(
    cur: &mut Cursor<'a>,
    ctx: &ObjectContext<'a>,
    source: Source,
) -> Result<Tree> {
    let version = cur.u16()?;
    if !(16..=20).contains(&version) {
        return Err(Error::parse(
            cur.position().saturating_sub(2),
            format!("unsupported TTree version {version}"),
        ));
    }
    let mut named_payload = cur.checked_sub()?;
    let named = parse_tnamed(&mut named_payload)?;
    let _tattline = cur.checked_sub()?;
    let _tattfill = cur.checked_sub()?;
    let _tattmarker = cur.checked_sub()?;
    let entries = cur.i64()?;
    let _total_bytes = cur.i64()?;
    let _zip_bytes = cur.i64()?;
    let _saved_bytes = cur.i64()?;
    if version >= 18 {
        let _flushed_bytes = cur.i64()?;
    }
    let _weight = cur.f64()?;
    let _timer_interval = cur.i32()?;
    let _scan_field = cur.i32()?;
    let _update = cur.i32()?;
    if version >= 17 {
        let _default_entry_offset_len = cur.i32()?;
    }
    let cluster_range_count = if version >= 19 {
        Some(cur.i32()?)
    } else {
        None
    };
    let _max_entries = cur.i64()?;
    let _max_entry_loop = cur.i64()?;
    let _max_virtual_size = cur.i64()?;
    let _auto_save = cur.i64()?;
    if version >= 18 {
        let _auto_flush = cur.i64()?;
    }
    let _estimate = cur.i64()?;
    if let Some(count) = cluster_range_count {
        let _marker = cur.u8()?;
        for _ in 0..count.max(0) {
            let _ = cur.i64()?;
        }
        let _marker = cur.u8()?;
        for _ in 0..count.max(0) {
            let _ = cur.i64()?;
        }
    }
    if version >= 20 {
        skip_tiofeatures(cur)?;
    }
    let mut branch_payload = cur.checked_sub()?;
    let branches = read_tobjarray(&mut branch_payload, ctx, |raw, ctx| {
        parse_branch_header(raw, ctx, source.clone())
    })?;
    let mut leaves_payload = cur.checked_sub()?;
    let _tree_leaves = read_tobjarray(&mut leaves_payload, ctx, parse_leaf)?;
    let _aliases = maybe_raw_buffer(cur, ctx)?;
    let index_value_count = cur.i32()?;
    for _ in 0..index_value_count.max(0) {
        let _ = cur.f64()?;
    }
    let index_count = cur.i32()?;
    for _ in 0..index_count.max(0) {
        let _ = cur.i32()?;
    }
    let _tree_index = maybe_raw_buffer(cur, ctx)?;
    let _friends = maybe_raw_buffer(cur, ctx)?;
    let _user_info = maybe_raw_buffer(cur, ctx)?;
    let _branch_ref = maybe_raw_buffer(cur, ctx)?;
    Ok(Tree {
        name: named.name,
        entries,
        branches,
    })
}

fn parse_branch_header<'a>(
    raw: RawObject<'a>,
    ctx: &ObjectContext<'a>,
    source: Source,
) -> Result<Branch> {
    match raw.class_name {
        "TBranchElement" | "TBranchObject" => {
            let mut cur = Cursor::with_origin(raw.payload, raw.payload_origin);
            let _version = cur.u16()?;
            let mut payload = cur.checked_sub()?;
            parse_branch(&mut payload, ctx, source)
        }
        "TBranch" => {
            let mut cur = Cursor::with_origin(raw.payload, raw.payload_origin);
            parse_branch(&mut cur, ctx, source)
        }
        other => Err(Error::parse(
            0,
            format!("unexpected branch object class {other}"),
        )),
    }
}

fn parse_branch<'a>(
    cur: &mut Cursor<'a>,
    ctx: &ObjectContext<'a>,
    source: Source,
) -> Result<Branch> {
    let version = cur.u16()?;
    if ![11, 12, 13].contains(&version) {
        return Err(Error::parse(
            cur.position().saturating_sub(2),
            format!("unsupported TBranch version {version}"),
        ));
    }
    let mut named_payload = cur.checked_sub()?;
    let named = parse_tnamed(&mut named_payload)?;
    let _tattfill = cur.checked_sub()?;
    let _compress = cur.i32()?;
    let _basket_size = cur.i32()?;
    let _entry_offset_len = cur.i32()?;
    let write_basket = cur.i32()?;
    let _entry_number = cur.i64()?;
    if version >= 13 {
        skip_tiofeatures(cur)?;
    }
    let _offset = cur.i32()?;
    let max_baskets = cur.i32()?;
    let _split_level = cur.i32()?;
    let entries = cur.i64()?;
    let _first_entry = cur.i64()?;
    let _total_bytes = cur.i64()?;
    let _zip_bytes = cur.i64()?;
    let mut branch_payload = cur.checked_sub()?;
    let branches = read_tobjarray(&mut branch_payload, ctx, |raw, ctx| {
        parse_branch_header(raw, ctx, source.clone())
    })?;
    let mut leaf_payload = cur.checked_sub()?;
    let leaves = read_tobjarray(&mut leaf_payload, ctx, parse_leaf)?;
    let mut basket_payload = cur.checked_sub()?;
    let memory_baskets = read_tobjarray(&mut basket_payload, ctx, |raw, _ctx| {
        Ok(raw.payload.to_vec())
    })?;
    let fbasketbytes = read_i32_array(cur, max_baskets)?;
    let fbasketentry = read_i64_array(cur, max_baskets)?;
    let fbasketseek = read_u64_array(cur, max_baskets)?;
    let file_name = cur.string()?;
    if !file_name.is_empty() {
        return Err(Error::unsupported(
            &named.name,
            "baskets stored in external files are not implemented",
        ));
    }
    let basket_count = write_basket.max(0) as usize;
    let mut baskets = memory_baskets
        .into_iter()
        .filter(|bytes| !bytes.is_empty())
        .map(BasketStorage::InMemory)
        .collect::<Vec<_>>();
    for (seek, len) in fbasketseek
        .into_iter()
        .zip(fbasketbytes.into_iter())
        .take(basket_count)
    {
        if seek != 0 && len > 0 {
            baskets.push(BasketStorage::OnDisk {
                source: source.clone(),
                seek,
                len: len as u64,
            });
        }
    }
    Ok(Branch {
        name: named.name,
        entries,
        branches,
        leaves,
        basket_entry: fbasketentry.into_iter().take(basket_count).collect(),
        baskets,
    })
}

fn read_i32_array(cur: &mut Cursor<'_>, count: i32) -> Result<Vec<i32>> {
    let _marker = cur.u8()?;
    let mut out = Vec::with_capacity(count.max(0) as usize);
    for _ in 0..count.max(0) {
        out.push(cur.i32()?);
    }
    Ok(out)
}

fn read_i64_array(cur: &mut Cursor<'_>, count: i32) -> Result<Vec<i64>> {
    let _marker = cur.u8()?;
    let mut out = Vec::with_capacity(count.max(0) as usize);
    for _ in 0..count.max(0) {
        out.push(cur.i64()?);
    }
    Ok(out)
}

fn read_u64_array(cur: &mut Cursor<'_>, count: i32) -> Result<Vec<u64>> {
    let _marker = cur.u8()?;
    let mut out = Vec::with_capacity(count.max(0) as usize);
    for _ in 0..count.max(0) {
        out.push(cur.u64()?);
    }
    Ok(out)
}

fn parse_leaf<'a>(raw: RawObject<'a>, ctx: &ObjectContext<'a>) -> Result<LeafInfo> {
    let mut cur = Cursor::with_origin(raw.payload, raw.payload_origin);
    match raw.class_name {
        "TLeafB" => parse_simple_leaf(&mut cur, ctx, raw.class_name, "i8", "u8", |cur| {
            cur.i8().map(|_| ())
        }),
        "TLeafS" => parse_simple_leaf(&mut cur, ctx, raw.class_name, "i16", "u16", |cur| {
            cur.i16().map(|_| ())
        }),
        "TLeafI" => parse_simple_leaf(&mut cur, ctx, raw.class_name, "i32", "u32", |cur| {
            cur.i32().map(|_| ())
        }),
        "TLeafL" => parse_simple_leaf(&mut cur, ctx, raw.class_name, "i64", "u64", |cur| {
            cur.i64().map(|_| ())
        }),
        "TLeafF" => parse_simple_leaf(&mut cur, ctx, raw.class_name, "f32", "f32", |cur| {
            cur.f32().map(|_| ())
        }),
        "TLeafD" => parse_simple_leaf(&mut cur, ctx, raw.class_name, "f64", "f64", |cur| {
            cur.f64().map(|_| ())
        }),
        "TLeafC" => parse_simple_leaf(&mut cur, ctx, raw.class_name, "String", "String", |cur| {
            cur.i32().map(|_| ())
        }),
        "TLeafO" => parse_simple_leaf(&mut cur, ctx, raw.class_name, "bool", "bool", |cur| {
            cur.bool().map(|_| ())
        }),
        "TLeafD32" => parse_simple_leaf(&mut cur, ctx, raw.class_name, "f32", "f32", |cur| {
            cur.f32().map(|_| ())
        }),
        "TLeafElement" => parse_leaf_element(&mut cur, ctx, raw.class_name),
        other => Err(Error::parse(0, format!("unexpected leaf class {other}"))),
    }
}

fn parse_simple_leaf<'a, F>(
    cur: &mut Cursor<'a>,
    ctx: &ObjectContext<'a>,
    root_class: &str,
    signed_type: &str,
    unsigned_type: &str,
    mut parse_min_max: F,
) -> Result<LeafInfo>
where
    F: FnMut(&mut Cursor<'_>) -> Result<()>,
{
    let version = cur.u16()?;
    if version != 1 {
        return Err(Error::parse(
            cur.position().saturating_sub(2),
            format!("unsupported {root_class} version {version}"),
        ));
    }
    let base = parse_leaf_base(cur, ctx)?;
    parse_min_max(cur)?;
    parse_min_max(cur)?;
    let type_name = if base.is_unsigned {
        unsigned_type
    } else {
        signed_type
    };
    Ok(LeafInfo {
        name: base.name,
        root_class: root_class.to_string(),
        type_name: arrayfy_maybe(type_name, base.len),
        len: base.len,
        element_size: base.element_size,
        is_unsigned: base.is_unsigned,
        has_leaf_count: base.leaf_count_name.is_some(),
        leaf_count_name: base.leaf_count_name,
    })
}

fn parse_leaf_element<'a>(
    cur: &mut Cursor<'a>,
    ctx: &ObjectContext<'a>,
    root_class: &str,
) -> Result<LeafInfo> {
    let version = cur.u16()?;
    if version != 1 {
        return Err(Error::parse(
            cur.position().saturating_sub(2),
            format!("unsupported TLeafElement version {version}"),
        ));
    }
    let base = parse_leaf_base(cur, ctx)?;
    let _id = cur.i32()?;
    let type_id = cur.i32()?;
    let type_name = primitive_type_name(type_id).unwrap_or("unsupported");
    Ok(LeafInfo {
        name: base.name,
        root_class: root_class.to_string(),
        type_name: arrayfy_maybe(type_name, base.len),
        len: base.len,
        element_size: base.element_size,
        is_unsigned: matches!(type_name, "u8" | "u16" | "u32" | "u64"),
        has_leaf_count: base.leaf_count_name.is_some(),
        leaf_count_name: base.leaf_count_name,
    })
}

struct LeafBase {
    name: String,
    len: i32,
    element_size: i32,
    is_unsigned: bool,
    leaf_count_name: Option<String>,
}

fn parse_leaf_base<'a>(cur: &mut Cursor<'a>, ctx: &ObjectContext<'a>) -> Result<LeafBase> {
    let mut base_payload = cur.checked_sub()?;
    let _version = base_payload.u16()?;
    let mut named_payload = base_payload.checked_sub()?;
    let named = parse_tnamed(&mut named_payload)?;
    let len = base_payload.i32()?;
    let element_size = base_payload.i32()?;
    let _offset = base_payload.i32()?;
    let _is_range = base_payload.bool()?;
    let is_unsigned = base_payload.bool()?;
    let leaf_count_name = read_raw_optional(&mut base_payload, ctx)?
        .map(|raw| parse_leaf(raw, ctx).map(|leaf| leaf.name))
        .transpose()?;
    Ok(LeafBase {
        name: named.name,
        len,
        element_size,
        is_unsigned,
        leaf_count_name,
    })
}

fn arrayfy_maybe(type_name: &str, len: i32) -> String {
    if len == 1 {
        type_name.to_string()
    } else {
        format!("[{type_name}; {len}]")
    }
}

fn primitive_type_name(type_id: i32) -> Option<&'static str> {
    match type_id {
        1 => Some("i8"),
        2 => Some("i16"),
        3 | 6 => Some("i32"),
        4 | 16 => Some("i64"),
        5 | 9 => Some("f32"),
        8 => Some("f64"),
        11 => Some("u8"),
        12 => Some("u16"),
        13 | 15 => Some("u32"),
        14 | 17 => Some("u64"),
        18 => Some("bool"),
        _ => None,
    }
}

fn read_basket_payload(storage: &BasketStorage) -> Result<(u32, Vec<u8>)> {
    let bytes = match storage {
        BasketStorage::InMemory(bytes) => bytes.clone(),
        BasketStorage::OnDisk { source, seek, len } => source.fetch(*seek, *len)?,
    };
    let mut cur = Cursor::new(&bytes);
    let key = parse_tkey_header(&mut cur)?;
    let _version = cur.u16()?;
    let _buffer_size = cur.u32()?;
    let _entry_size = cur.u32()?;
    let entries = cur.u32()?;
    let last = cur.u32()?;
    let _flag = cur.i8()?;
    let payload = cur.rest();
    let payload = if key.uncompressed_len as usize > payload.len() {
        decompress_root_blocks(payload)?
    } else {
        payload.to_vec()
    };
    let useful_len = last
        .checked_sub(key.key_len as u32)
        .ok_or_else(|| Error::parse(0, "basket last byte precedes key length"))?
        as usize;
    if useful_len > payload.len() {
        return Err(Error::parse(
            payload.len(),
            format!("basket useful length {useful_len} exceeds payload length"),
        ));
    }
    Ok((entries, payload[..useful_len].to_vec()))
}
