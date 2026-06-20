use std::fmt::Debug;

use crate::decompress::decompress_root_blocks;
use crate::error::{Error, Result};
use crate::parse::{
    maybe_raw_buffer, parse_tnamed, read_raw, read_tobjarray, skip_tiofeatures, Cursor,
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
        let branch = self
            .branch_refs()
            .into_iter()
            .find(|branch| branch.name == branch_name)
            .ok_or_else(|| Error::MissingBranch(branch_name.to_string()))?;
        let leaf = branch.scalar_leaf()?;
        if leaf.type_name != T::TYPE_NAME {
            return Err(Error::TypeMismatch {
                branch: branch_name.to_string(),
                root_type: leaf.type_name.clone(),
                requested: T::TYPE_NAME,
            });
        }
        let mut out = Vec::with_capacity(branch.entries.max(0) as usize);
        for basket in &branch.baskets {
            let (entries, payload) = read_basket_payload(basket)?;
            let need = entries as usize * T::WIDTH;
            if payload.len() < need {
                return Err(Error::parse(
                    payload.len(),
                    format!("basket for {branch_name} needs {need} bytes"),
                ));
            }
            for chunk in payload[..need].chunks_exact(T::WIDTH) {
                out.push(T::decode(chunk));
            }
        }
        out.truncate(branch.entries.max(0) as usize);
        Ok(out)
    }

    fn branch_refs(&self) -> Vec<&Branch> {
        let mut out = Vec::new();
        for branch in &self.branches {
            branch.collect_refs(&mut out);
        }
        out
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
                "jagged leaf-count branches are P2 scope",
            ));
        }
        Ok(leaf)
    }
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
            let mut cur = Cursor::new(raw.payload);
            let _version = cur.u16()?;
            let mut payload = cur.checked_sub()?;
            parse_branch(&mut payload, ctx, source)
        }
        "TBranch" => {
            let mut cur = Cursor::new(raw.payload);
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
    let mut cur = Cursor::new(raw.payload);
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
        has_leaf_count: base.has_leaf_count,
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
        has_leaf_count: base.has_leaf_count,
    })
}

struct LeafBase {
    name: String,
    len: i32,
    element_size: i32,
    is_unsigned: bool,
    has_leaf_count: bool,
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
    let has_leaf_count = if base_payload.peek_u32()? == 0 {
        let _ = base_payload.u32()?;
        false
    } else {
        let _ = read_raw(&mut base_payload, ctx)?;
        true
    };
    Ok(LeafBase {
        name: named.name,
        len,
        element_size,
        is_unsigned,
        has_leaf_count,
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
