use std::convert::TryFrom;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use crate::error::{Error, Result};
use crate::parse::TBUFFER_OBJECT_MAP_OFFSET;

const FILE_BEGIN: u32 = 100;
const DIRECTORY_OFFSET: u64 = FILE_BEGIN as u64;
const DIRECTORY_SIZE: u64 = 30;
const TREE_OFFSET: u64 = DIRECTORY_OFFSET + DIRECTORY_SIZE;
const STOCK_DIRECTORY_SIZE: usize = 60;

#[derive(Debug, Clone)]
pub struct Branch {
    name: String,
    data: BranchData,
    counter_name: Option<String>,
}

#[derive(Debug, Clone)]
pub enum BranchData {
    U8(Vec<u8>),
    I8(Vec<i8>),
    U16(Vec<u16>),
    I16(Vec<i16>),
    U32(Vec<u32>),
    I32(Vec<i32>),
    U64(Vec<u64>),
    I64(Vec<i64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
    Bool(Vec<bool>),
    VecU8(Vec<Vec<u8>>),
    VecI8(Vec<Vec<i8>>),
    VecU16(Vec<Vec<u16>>),
    VecI16(Vec<Vec<i16>>),
    VecU32(Vec<Vec<u32>>),
    VecI32(Vec<Vec<i32>>),
    VecU64(Vec<Vec<u64>>),
    VecI64(Vec<Vec<i64>>),
    VecF32(Vec<Vec<f32>>),
    VecF64(Vec<Vec<f64>>),
}

/// Axis binning for a writable ROOT `TH1F`.
#[derive(Debug, Clone, PartialEq)]
pub enum HistogramAxis {
    /// Uniform bins over `[low, high)`.
    Fixed { bins: usize, low: f64, high: f64 },
    /// Explicit bin edges. Length must be `nbins + 1`.
    Variable { edges: Vec<f64> },
}

impl HistogramAxis {
    fn validate(&self, name: &str) -> Result<()> {
        match self {
            Self::Fixed { bins, low, high } => {
                if *bins == 0 {
                    return Err(Error::unsupported(
                        name,
                        "histogram must have at least one bin",
                    ));
                }
                if !(low.is_finite() && high.is_finite() && high > low) {
                    return Err(Error::unsupported(
                        name,
                        "fixed histogram bounds must be finite and ordered",
                    ));
                }
            }
            Self::Variable { edges } => {
                if edges.len() < 2 {
                    return Err(Error::unsupported(
                        name,
                        "variable histogram needs at least two edges",
                    ));
                }
                if edges
                    .windows(2)
                    .any(|pair| !pair[0].is_finite() || !pair[1].is_finite() || pair[1] <= pair[0])
                {
                    return Err(Error::unsupported(
                        name,
                        "variable histogram edges must be finite and strictly increasing",
                    ));
                }
            }
        }
        Ok(())
    }

    fn bins(&self) -> usize {
        match self {
            Self::Fixed { bins, .. } => *bins,
            Self::Variable { edges } => edges.len() - 1,
        }
    }

    fn low(&self) -> f64 {
        match self {
            Self::Fixed { low, .. } => *low,
            Self::Variable { edges } => edges[0],
        }
    }

    fn high(&self) -> f64 {
        match self {
            Self::Fixed { high, .. } => *high,
            Self::Variable { edges } => edges[edges.len() - 1],
        }
    }

    fn edges(&self) -> &[f64] {
        match self {
            Self::Fixed { .. } => &[],
            Self::Variable { edges } => edges,
        }
    }
}

/// Writable one-dimensional histogram payload for a ROOT `TH1F` key.
#[derive(Debug, Clone, PartialEq)]
pub struct Th1F {
    name: String,
    title: String,
    axis: HistogramAxis,
    contents: Vec<f64>,
    sumw2: Vec<f64>,
    entries: f64,
    tsumwx: f64,
    tsumwx2: f64,
}

impl Th1F {
    /// Build a `TH1F`.
    ///
    /// `contents` and `sumw2` include underflow at index 0 and overflow at
    /// index `nbins + 1`.
    pub fn new(
        name: impl Into<String>,
        title: impl Into<String>,
        axis: HistogramAxis,
        contents: Vec<f64>,
        sumw2: Vec<f64>,
        entries: f64,
    ) -> Self {
        Self {
            name: name.into(),
            title: title.into(),
            axis,
            contents,
            sumw2,
            entries,
            tsumwx: 0.0,
            tsumwx2: 0.0,
        }
    }

    /// Attach exact in-range weighted x statistics.
    pub fn with_weighted_x_stats(mut self, sumwx: f64, sumwx2: f64) -> Self {
        self.tsumwx = sumwx;
        self.tsumwx2 = sumwx2;
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    fn validate(&self) -> Result<()> {
        if self.name.is_empty() {
            return Err(Error::unsupported(
                "TH1F writer",
                "histogram names must not be empty",
            ));
        }
        self.axis.validate(&self.name)?;
        let ncells = self.axis.bins() + 2;
        if self.contents.len() != ncells {
            return Err(Error::unsupported(
                &self.name,
                format!(
                    "contents length is {}, expected nbins + 2 = {ncells}",
                    self.contents.len()
                ),
            ));
        }
        if self.sumw2.len() != ncells {
            return Err(Error::unsupported(
                &self.name,
                format!(
                    "sumw2 length is {}, expected nbins + 2 = {ncells}",
                    self.sumw2.len()
                ),
            ));
        }
        if self
            .contents
            .iter()
            .any(|value| !value.is_finite() || *value < f32::MIN as f64 || *value > f32::MAX as f64)
        {
            return Err(Error::unsupported(
                &self.name,
                "TH1F bin contents must be finite and fit in f32",
            ));
        }
        if self
            .sumw2
            .iter()
            .chain([self.entries, self.tsumwx, self.tsumwx2].iter())
            .any(|value| !value.is_finite())
        {
            return Err(Error::unsupported(
                &self.name,
                "histogram statistics must be finite",
            ));
        }
        Ok(())
    }
}

macro_rules! branch_ctor {
    ($name:ident, $variant:ident, $ty:ty) => {
        pub fn $name(name: impl Into<String>, values: Vec<$ty>) -> Self {
            Self {
                name: name.into(),
                data: BranchData::$variant(values),
                counter_name: None,
            }
        }
    };
}

macro_rules! branch_vec_ctor {
    ($name:ident, $variant:ident, $ty:ty) => {
        pub fn $name(name: impl Into<String>, values: Vec<Vec<$ty>>) -> Self {
            Self {
                name: name.into(),
                data: BranchData::$variant(values),
                counter_name: None,
            }
        }
    };
}

impl Branch {
    branch_ctor!(u8, U8, u8);
    branch_ctor!(i8, I8, i8);
    branch_ctor!(u16, U16, u16);
    branch_ctor!(i16, I16, i16);
    branch_ctor!(u32, U32, u32);
    branch_ctor!(i32, I32, i32);
    branch_ctor!(u64, U64, u64);
    branch_ctor!(i64, I64, i64);
    branch_ctor!(f32, F32, f32);
    branch_ctor!(f64, F64, f64);
    branch_ctor!(bool, Bool, bool);

    branch_vec_ctor!(vec_u8, VecU8, u8);
    branch_vec_ctor!(vec_i8, VecI8, i8);
    branch_vec_ctor!(vec_u16, VecU16, u16);
    branch_vec_ctor!(vec_i16, VecI16, i16);
    branch_vec_ctor!(vec_u32, VecU32, u32);
    branch_vec_ctor!(vec_i32, VecI32, i32);
    branch_vec_ctor!(vec_u64, VecU64, u64);
    branch_vec_ctor!(vec_i64, VecI64, i64);
    branch_vec_ctor!(vec_f32, VecF32, f32);
    branch_vec_ctor!(vec_f64, VecF64, f64);

    pub fn with_counter_name(mut self, counter_name: impl Into<String>) -> Self {
        self.counter_name = Some(counter_name.into());
        self
    }
}

impl BranchData {
    fn len(&self) -> usize {
        match self {
            Self::U8(v) => v.len(),
            Self::I8(v) => v.len(),
            Self::U16(v) => v.len(),
            Self::I16(v) => v.len(),
            Self::U32(v) => v.len(),
            Self::I32(v) => v.len(),
            Self::U64(v) => v.len(),
            Self::I64(v) => v.len(),
            Self::F32(v) => v.len(),
            Self::F64(v) => v.len(),
            Self::Bool(v) => v.len(),
            Self::VecU8(v) => v.len(),
            Self::VecI8(v) => v.len(),
            Self::VecU16(v) => v.len(),
            Self::VecI16(v) => v.len(),
            Self::VecU32(v) => v.len(),
            Self::VecI32(v) => v.len(),
            Self::VecU64(v) => v.len(),
            Self::VecI64(v) => v.len(),
            Self::VecF32(v) => v.len(),
            Self::VecF64(v) => v.len(),
        }
    }

    fn payload(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Self::U8(values) => values.iter().for_each(|value| put_u8(&mut out, *value)),
            Self::I8(values) => values.iter().for_each(|value| put_i8(&mut out, *value)),
            Self::U16(values) => values.iter().for_each(|value| put_u16(&mut out, *value)),
            Self::I16(values) => values.iter().for_each(|value| put_i16(&mut out, *value)),
            Self::U32(values) => values.iter().for_each(|value| put_u32(&mut out, *value)),
            Self::I32(values) => values.iter().for_each(|value| put_i32(&mut out, *value)),
            Self::U64(values) => values.iter().for_each(|value| put_u64(&mut out, *value)),
            Self::I64(values) => values.iter().for_each(|value| put_i64(&mut out, *value)),
            Self::F32(values) => values.iter().for_each(|value| put_f32(&mut out, *value)),
            Self::F64(values) => values.iter().for_each(|value| put_f64(&mut out, *value)),
            Self::Bool(values) => values
                .iter()
                .for_each(|value| put_u8(&mut out, u8::from(*value))),
            Self::VecU8(rows) => rows
                .iter()
                .flatten()
                .for_each(|value| put_u8(&mut out, *value)),
            Self::VecI8(rows) => rows
                .iter()
                .flatten()
                .for_each(|value| put_i8(&mut out, *value)),
            Self::VecU16(rows) => rows
                .iter()
                .flatten()
                .for_each(|value| put_u16(&mut out, *value)),
            Self::VecI16(rows) => rows
                .iter()
                .flatten()
                .for_each(|value| put_i16(&mut out, *value)),
            Self::VecU32(rows) => rows
                .iter()
                .flatten()
                .for_each(|value| put_u32(&mut out, *value)),
            Self::VecI32(rows) => rows
                .iter()
                .flatten()
                .for_each(|value| put_i32(&mut out, *value)),
            Self::VecU64(rows) => rows
                .iter()
                .flatten()
                .for_each(|value| put_u64(&mut out, *value)),
            Self::VecI64(rows) => rows
                .iter()
                .flatten()
                .for_each(|value| put_i64(&mut out, *value)),
            Self::VecF32(rows) => rows
                .iter()
                .flatten()
                .for_each(|value| put_f32(&mut out, *value)),
            Self::VecF64(rows) => rows
                .iter()
                .flatten()
                .for_each(|value| put_f64(&mut out, *value)),
        }
        out
    }

    fn is_jagged(&self) -> bool {
        matches!(
            self,
            Self::VecU8(_)
                | Self::VecI8(_)
                | Self::VecU16(_)
                | Self::VecI16(_)
                | Self::VecU32(_)
                | Self::VecI32(_)
                | Self::VecU64(_)
                | Self::VecI64(_)
                | Self::VecF32(_)
                | Self::VecF64(_)
        )
    }

    fn row_lengths(&self) -> Option<Vec<usize>> {
        match self {
            Self::VecU8(rows) => Some(rows.iter().map(Vec::len).collect()),
            Self::VecI8(rows) => Some(rows.iter().map(Vec::len).collect()),
            Self::VecU16(rows) => Some(rows.iter().map(Vec::len).collect()),
            Self::VecI16(rows) => Some(rows.iter().map(Vec::len).collect()),
            Self::VecU32(rows) => Some(rows.iter().map(Vec::len).collect()),
            Self::VecI32(rows) => Some(rows.iter().map(Vec::len).collect()),
            Self::VecU64(rows) => Some(rows.iter().map(Vec::len).collect()),
            Self::VecI64(rows) => Some(rows.iter().map(Vec::len).collect()),
            Self::VecF32(rows) => Some(rows.iter().map(Vec::len).collect()),
            Self::VecF64(rows) => Some(rows.iter().map(Vec::len).collect()),
            _ => None,
        }
    }

    fn leaf_class(&self) -> &'static str {
        match self {
            Self::U8(_) | Self::I8(_) | Self::VecU8(_) | Self::VecI8(_) => "TLeafB",
            Self::U16(_) | Self::I16(_) | Self::VecU16(_) | Self::VecI16(_) => "TLeafS",
            Self::U32(_) | Self::I32(_) | Self::VecU32(_) | Self::VecI32(_) => "TLeafI",
            Self::U64(_) | Self::I64(_) | Self::VecU64(_) | Self::VecI64(_) => "TLeafL",
            Self::F32(_) | Self::VecF32(_) => "TLeafF",
            Self::F64(_) | Self::VecF64(_) => "TLeafD",
            Self::Bool(_) => "TLeafO",
        }
    }

    fn leaflist_code(&self) -> &'static str {
        match self {
            Self::I8(_) | Self::VecI8(_) => "B",
            Self::U8(_) | Self::VecU8(_) => "b",
            Self::I16(_) | Self::VecI16(_) => "S",
            Self::U16(_) | Self::VecU16(_) => "s",
            Self::I32(_) | Self::VecI32(_) => "I",
            Self::U32(_) | Self::VecU32(_) => "i",
            Self::I64(_) | Self::VecI64(_) => "L",
            Self::U64(_) | Self::VecU64(_) => "l",
            Self::F32(_) | Self::VecF32(_) => "F",
            Self::F64(_) | Self::VecF64(_) => "D",
            Self::Bool(_) => "O",
        }
    }

    fn branch_title(&self, name: &str, counter_name: Option<&str>) -> String {
        match counter_name {
            Some(counter_name) => format!("{name}[{counter_name}]/{}", self.leaflist_code()),
            None => format!("{name}/{}", self.leaflist_code()),
        }
    }

    fn leaf_title(&self, name: &str, counter_name: Option<&str>) -> String {
        match counter_name {
            Some(counter_name) => format!("{name}[{counter_name}]"),
            None => name.to_string(),
        }
    }

    fn element_size(&self) -> i32 {
        match self {
            Self::U8(_) | Self::I8(_) | Self::Bool(_) | Self::VecU8(_) | Self::VecI8(_) => 1,
            Self::U16(_) | Self::I16(_) | Self::VecU16(_) | Self::VecI16(_) => 2,
            Self::U32(_)
            | Self::I32(_)
            | Self::F32(_)
            | Self::VecU32(_)
            | Self::VecI32(_)
            | Self::VecF32(_) => 4,
            Self::U64(_)
            | Self::I64(_)
            | Self::F64(_)
            | Self::VecU64(_)
            | Self::VecI64(_)
            | Self::VecF64(_) => 8,
        }
    }

    fn is_unsigned(&self) -> bool {
        matches!(
            self,
            Self::U8(_)
                | Self::U16(_)
                | Self::U32(_)
                | Self::U64(_)
                | Self::VecU8(_)
                | Self::VecU16(_)
                | Self::VecU32(_)
                | Self::VecU64(_)
        )
    }

    fn entry_offset_len(&self, entries: usize) -> i32 {
        if self.is_jagged() {
            4 * entries as i32
        } else {
            0
        }
    }

    fn basket_uncompressed_len(&self) -> usize {
        let payload_len = self.payload().len();
        if self.is_jagged() {
            payload_len + 4 * (self.len() + 2)
        } else {
            payload_len
        }
    }

    fn write_min_max(&self, out: &mut Vec<u8>) {
        match self {
            Self::U8(values) => write_min_max_u8(out, values.iter().copied()),
            Self::I8(values) => write_min_max_i8(out, values.iter().copied()),
            Self::U16(values) => write_min_max_u16(out, values.iter().copied()),
            Self::I16(values) => write_min_max_i16(out, values.iter().copied()),
            Self::U32(values) => write_min_max_u32(out, values.iter().copied()),
            Self::I32(values) => write_min_max_i32(out, values.iter().copied()),
            Self::U64(values) => write_min_max_u64(out, values.iter().copied()),
            Self::I64(values) => write_min_max_i64(out, values.iter().copied()),
            Self::F32(values) => write_min_max_f32(out, values.iter().copied()),
            Self::F64(values) => write_min_max_f64(out, values.iter().copied()),
            Self::Bool(values) => write_min_max_bool(out, values.iter().copied()),
            Self::VecU8(rows) => write_min_max_u8(out, rows.iter().flatten().copied()),
            Self::VecI8(rows) => write_min_max_i8(out, rows.iter().flatten().copied()),
            Self::VecU16(rows) => write_min_max_u16(out, rows.iter().flatten().copied()),
            Self::VecI16(rows) => write_min_max_i16(out, rows.iter().flatten().copied()),
            Self::VecU32(rows) => write_min_max_u32(out, rows.iter().flatten().copied()),
            Self::VecI32(rows) => write_min_max_i32(out, rows.iter().flatten().copied()),
            Self::VecU64(rows) => write_min_max_u64(out, rows.iter().flatten().copied()),
            Self::VecI64(rows) => write_min_max_i64(out, rows.iter().flatten().copied()),
            Self::VecF32(rows) => write_min_max_f32(out, rows.iter().flatten().copied()),
            Self::VecF64(rows) => write_min_max_f64(out, rows.iter().flatten().copied()),
        }
    }
}

#[derive(Debug, Clone)]
struct BasketInfo {
    bytes: Vec<u8>,
    uncompressed_len: usize,
    seek: u64,
}

#[derive(Debug, Clone)]
struct BranchMeta {
    counter: Option<usize>,
    is_counter: bool,
}

#[derive(Debug, Clone)]
struct TKeySpec {
    total_size: u32,
    version: u16,
    uncomp_len: u32,
    datime: u32,
    key_len: i16,
    cycle: i16,
    seek_key: u64,
    seek_pdir: u64,
    class_name: String,
    obj_name: String,
    obj_title: String,
}

fn build_branch_meta(branches: &[Branch]) -> Result<Vec<BranchMeta>> {
    let mut meta = vec![
        BranchMeta {
            counter: None,
            is_counter: false,
        };
        branches.len()
    ];

    for (branch_index, branch) in branches.iter().enumerate() {
        if !branch.data.is_jagged() {
            continue;
        }
        let counter_name = counter_name_for_branch(branch).ok_or_else(|| {
            Error::unsupported(
                &branch.name,
                "jagged branches need a NanoAOD-style `Prefix_attr` name",
            )
        })?;
        let counter_index = branches[..branch_index]
            .iter()
            .position(|candidate| candidate.name == counter_name)
            .ok_or_else(|| {
                Error::unsupported(
                    &branch.name,
                    format!("jagged branch needs earlier counter branch `{counter_name}`"),
                )
            })?;
        let counts = counter_lengths(&branch.name, &counter_name, &branches[counter_index].data)?;
        let lengths = branch.data.row_lengths().unwrap_or_default();
        for (entry, (&row_len, count)) in lengths.iter().zip(counts).enumerate() {
            if row_len != count {
                return Err(Error::unsupported(
                    &branch.name,
                    format!("entry {entry} has {row_len} values but `{counter_name}` is {count}"),
                ));
            }
        }
        meta[branch_index].counter = Some(counter_index);
        meta[counter_index].is_counter = true;
    }

    Ok(meta)
}

fn counter_lengths(branch_name: &str, counter_name: &str, data: &BranchData) -> Result<Vec<usize>> {
    match data {
        BranchData::U32(counts) => Ok(counts.iter().map(|&count| count as usize).collect()),
        BranchData::I32(counts) => counts
            .iter()
            .map(|&count| {
                usize::try_from(count).map_err(|_| {
                    Error::unsupported(
                        branch_name,
                        format!("counter branch `{counter_name}` contains negative count {count}"),
                    )
                })
            })
            .collect(),
        _ => Err(Error::unsupported(
            branch_name,
            format!("counter branch `{counter_name}` must be UInt_t/u32 or Int_t/i32"),
        )),
    }
}

fn counter_name_for_branch(branch: &Branch) -> Option<String> {
    branch
        .counter_name
        .clone()
        .or_else(|| counter_name_for(&branch.name))
}

fn counter_name_for(branch_name: &str) -> Option<String> {
    branch_name
        .split_once('_')
        .and_then(|(prefix, _)| (!prefix.is_empty()).then(|| format!("n{prefix}")))
}

/// Write an uncompressed ROOT file containing one `TTree`.
///
/// This covers the NanoAOD subset used by the owned reader: scalar primitive
/// leaves, NanoAOD jagged-by-counter leaves, and one uncompressed `TBasket` per
/// branch. Jagged leaves serialize `TLeaf::fLeafCount` as a ROOT object
/// back-reference to the earlier counter leaf, not as embedded counter data.
pub fn write_tree<P: AsRef<Path>>(path: P, tree_name: &str, branches: &[Branch]) -> Result<()> {
    if branches.is_empty() {
        return Err(Error::unsupported(
            "TTree writer",
            "cannot write no branches",
        ));
    }

    let entries = branches[0].data.len();
    for branch in branches {
        if branch.name.is_empty() {
            return Err(Error::unsupported(
                "TTree writer",
                "branch names must not be empty",
            ));
        }
        if branch.data.len() != entries {
            return Err(Error::unsupported(
                &branch.name,
                format!("has {} entries, expected {entries}", branch.data.len()),
            ));
        }
        if matches!(branch.data, BranchData::Bool(_)) && branch.data.is_jagged() {
            return Err(Error::unsupported(
                &branch.name,
                "jagged bool is not implemented",
            ));
        }
    }

    let branch_meta = build_branch_meta(branches)?;

    let mut baskets: Vec<BasketInfo> = branches
        .iter()
        .map(|branch| {
            let payload = branch.data.payload();
            let bytes = build_basket(branch, tree_name, 0, &payload, entries);
            BasketInfo {
                bytes,
                uncompressed_len: branch.data.basket_uncompressed_len(),
                seek: 0,
            }
        })
        .collect();

    let provisional_tree_obj =
        build_tree_object(tree_name, branches, &branch_meta, &baskets, entries)?;
    let provisional_tree_key = build_key(
        "TTree",
        tree_name,
        tree_name,
        TREE_OFFSET,
        &provisional_tree_obj,
    )?;
    let mut next_seek = TREE_OFFSET + provisional_tree_key.len() as u64;

    for (branch, basket) in branches.iter().zip(baskets.iter_mut()) {
        basket.seek = next_seek;
        basket.bytes = build_basket(
            branch,
            tree_name,
            basket.seek,
            &branch.data.payload(),
            entries,
        );
        next_seek += basket.bytes.len() as u64;
    }

    let tree_obj = build_tree_object(tree_name, branches, &branch_meta, &baskets, entries)?;
    let tree_key = build_key("TTree", tree_name, tree_name, TREE_OFFSET, &tree_obj)?;
    let tree_key_header = key_spec(
        "TTree",
        tree_name,
        tree_name,
        TREE_OFFSET,
        tree_obj.len(),
        false,
    )?;

    let key_list_offset = TREE_OFFSET
        + tree_key.len() as u64
        + baskets.iter().map(|b| b.bytes.len() as u64).sum::<u64>();
    let key_list_obj = build_key_list(&[tree_key_header]);
    let key_list_key = build_key(
        "TFile",
        "nano.rust",
        "nano.rust",
        key_list_offset,
        &key_list_obj,
    )?;

    let streamer_info_offset = key_list_offset + key_list_key.len() as u64;
    let streamer_info_obj = checked(empty_tlist())?;
    let streamer_info_key = build_key(
        "TList",
        "StreamerInfo",
        "Doubly linked list",
        streamer_info_offset,
        &streamer_info_obj,
    )?;

    let file_end = streamer_info_offset + streamer_info_key.len() as u64 + 4;
    let mut file_bytes = vec![0; FILE_BEGIN as usize];
    write_file_header(
        &mut file_bytes[..75],
        u32::try_from(file_end).map_err(|_| Error::unsupported("TFile", "file too large"))?,
        u32::try_from(streamer_info_offset)
            .map_err(|_| Error::unsupported("TFile", "streamer info offset too large"))?,
        u32::try_from(streamer_info_key.len())
            .map_err(|_| Error::unsupported("TFile", "streamer info key too large"))?,
    );
    file_bytes.extend(build_directory(
        u32::try_from(key_list_offset)
            .map_err(|_| Error::unsupported("TFile", "key list offset too large"))?,
        u32::try_from(key_list_key.len())
            .map_err(|_| Error::unsupported("TFile", "key list too large"))?,
    ));
    file_bytes.extend(tree_key);
    for basket in &baskets {
        file_bytes.extend(&basket.bytes);
    }
    file_bytes.extend(key_list_key);
    file_bytes.extend(streamer_info_key);
    file_bytes.extend([0, 0, 0, 0]);

    let mut file = File::create(path)?;
    file.write_all(&file_bytes)?;
    Ok(())
}

/// Write an uncompressed ROOT file containing one `TH1F` key per histogram.
pub fn write_histograms<P: AsRef<Path>>(path: P, histograms: &[Th1F]) -> Result<()> {
    if histograms.is_empty() {
        return Err(Error::unsupported(
            "TH1F writer",
            "cannot write no histograms",
        ));
    }
    for histogram in histograms {
        histogram.validate()?;
    }

    let file_name = "nano.rust";
    let tfile_key_len = tfile_directory_key_len(file_name);
    let nbytes_name = tfile_nbytes_name(file_name);

    let mut object_keys = Vec::with_capacity(histograms.len());
    let mut object_headers = Vec::with_capacity(histograms.len());
    let mut next_seek = FILE_BEGIN as u64 + tfile_key_len as u64;
    for histogram in histograms {
        let object = build_th1f_object(histogram)?;
        let key = build_key(
            "TH1F",
            &histogram.name,
            &histogram.title,
            next_seek,
            &object,
        )?;
        let header = key_spec(
            "TH1F",
            &histogram.name,
            &histogram.title,
            next_seek,
            object.len(),
            false,
        )?;
        next_seek += key.len() as u64;
        object_keys.push(key);
        object_headers.push(header);
    }

    let key_list_offset = next_seek;
    let key_list_obj = build_key_list(&object_headers);
    let key_list_key = build_key(
        "TFile",
        "nano.rust",
        "nano.rust",
        key_list_offset,
        &key_list_obj,
    )?;

    let streamer_info_offset = key_list_offset + key_list_key.len() as u64;
    let streamer_info_obj = checked(empty_tlist())?;
    let streamer_info_key = build_key(
        "TList",
        "StreamerInfo",
        "Doubly linked list",
        streamer_info_offset,
        &streamer_info_obj,
    )?;

    let file_end = streamer_info_offset + streamer_info_key.len() as u64 + 4;
    let mut file_bytes = vec![0; FILE_BEGIN as usize];
    write_file_header_with_nbytes_name(
        &mut file_bytes[..75],
        u32::try_from(file_end).map_err(|_| Error::unsupported("TFile", "file too large"))?,
        u32::try_from(nbytes_name)
            .map_err(|_| Error::unsupported("TFile", "TFile name too large"))?,
        u32::try_from(streamer_info_offset)
            .map_err(|_| Error::unsupported("TFile", "streamer info offset too large"))?,
        u32::try_from(streamer_info_key.len())
            .map_err(|_| Error::unsupported("TFile", "streamer info key too large"))?,
    );
    file_bytes.extend(build_tfile_directory_key(
        file_name,
        u32::try_from(key_list_offset)
            .map_err(|_| Error::unsupported("TFile", "key list offset too large"))?,
        u32::try_from(key_list_key.len())
            .map_err(|_| Error::unsupported("TFile", "key list too large"))?,
    )?);
    for key in object_keys {
        file_bytes.extend(key);
    }
    file_bytes.extend(key_list_key);
    file_bytes.extend(streamer_info_key);
    file_bytes.extend([0, 0, 0, 0]);

    let mut file = File::create(path)?;
    file.write_all(&file_bytes)?;
    Ok(())
}

fn write_file_header(out: &mut [u8], end: u32, seek_info: u32, nbytes_info: u32) {
    write_file_header_with_nbytes_name(out, end, 0, seek_info, nbytes_info);
}

fn write_file_header_with_nbytes_name(
    out: &mut [u8],
    end: u32,
    nbytes_name: u32,
    seek_info: u32,
    nbytes_info: u32,
) {
    let mut bytes = Vec::with_capacity(75);
    bytes.extend(b"root");
    put_i32(&mut bytes, 62400);
    put_i32(&mut bytes, FILE_BEGIN as i32);
    put_u32(&mut bytes, end);
    put_u32(&mut bytes, 0);
    put_i32(&mut bytes, 0);
    put_i32(&mut bytes, 0);
    put_i32(&mut bytes, nbytes_name as i32);
    put_u8(&mut bytes, 4);
    put_i32(&mut bytes, 0);
    put_u32(&mut bytes, seek_info);
    put_i32(&mut bytes, nbytes_info as i32);
    put_u16(&mut bytes, 1);
    bytes.extend([0; 16]);
    out[..bytes.len()].copy_from_slice(&bytes);
}

fn build_tfile_directory_key(
    file_name: &str,
    seek_keys: u32,
    n_bytes_keys: u32,
) -> Result<Vec<u8>> {
    let directory = build_stock_directory(file_name, seek_keys, n_bytes_keys)?;
    let object_len = root_c_string_len(file_name) + directory.len();
    let key_len = key_header_len("TFile", file_name, "");
    let spec = TKeySpec {
        total_size: u32::try_from(key_len + object_len)
            .map_err(|_| Error::unsupported("TFile", "directory key too large"))?,
        version: 1004,
        uncomp_len: u32::try_from(object_len)
            .map_err(|_| Error::unsupported("TFile", "directory payload too large"))?,
        datime: 0,
        key_len: i16::try_from(key_len)
            .map_err(|_| Error::unsupported("TFile", "directory key header too large"))?,
        cycle: 1,
        seek_key: FILE_BEGIN as u64,
        seek_pdir: 0,
        class_name: "TFile".to_string(),
        obj_name: file_name.to_string(),
        obj_title: String::new(),
    };
    let mut out = Vec::with_capacity(key_len + object_len);
    write_key_header(&mut out, &spec);
    put_c_string(&mut out, file_name);
    out.extend(directory);
    Ok(out)
}

fn tfile_nbytes_name(file_name: &str) -> usize {
    key_header_len("TFile", file_name, "") + root_c_string_len(file_name)
}

fn tfile_directory_key_len(file_name: &str) -> usize {
    tfile_nbytes_name(file_name) + STOCK_DIRECTORY_SIZE
}

fn build_stock_directory(file_name: &str, seek_keys: u32, n_bytes_keys: u32) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(STOCK_DIRECTORY_SIZE);
    put_i16(&mut out, 5);
    put_u32(&mut out, 0);
    put_u32(&mut out, 0);
    put_i32(&mut out, n_bytes_keys as i32);
    put_i32(
        &mut out,
        i32::try_from(tfile_nbytes_name(file_name))
            .map_err(|_| Error::unsupported("TFile", "TFile name too large"))?,
    );
    put_u32(&mut out, FILE_BEGIN);
    put_u32(&mut out, 0);
    put_u32(&mut out, seek_keys);
    out.extend([0; STOCK_DIRECTORY_SIZE - 30]);
    Ok(out)
}

fn build_directory(seek_keys: u32, n_bytes_keys: u32) -> Vec<u8> {
    let mut out = Vec::new();
    put_i16(&mut out, 5);
    put_u32(&mut out, 0);
    put_u32(&mut out, 0);
    put_i32(&mut out, n_bytes_keys as i32);
    put_i32(&mut out, 0);
    put_u32(&mut out, DIRECTORY_OFFSET as u32);
    put_u32(&mut out, 0);
    put_u32(&mut out, seek_keys);
    out
}

fn build_key_list(headers: &[TKeySpec]) -> Vec<u8> {
    let mut out = Vec::new();
    put_u32(&mut out, headers.len() as u32);
    for header in headers {
        write_key_header(&mut out, header);
    }
    out
}

fn build_th1f_object(histogram: &Th1F) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    put_u16(&mut out, 3);
    out.extend(checked(build_th1_base(histogram)?)?);
    out.extend(tarray_f(&histogram.contents)?);
    checked(out)
}

fn build_th1_base(histogram: &Th1F) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    put_u16(&mut out, 8);
    out.extend(checked(tnamed(&histogram.name, &histogram.title))?);
    out.extend(checked(tattline_v2())?);
    out.extend(checked(tattfill_v2())?);
    out.extend(checked(tattmarker_v2())?);

    let nbins = histogram.axis.bins();
    let ncells = nbins + 2;
    put_i32(
        &mut out,
        i32::try_from(ncells).map_err(|_| Error::unsupported(&histogram.name, "too many bins"))?,
    );
    out.extend(checked(taxis(
        "xaxis",
        "",
        nbins,
        histogram.axis.low(),
        histogram.axis.high(),
        histogram.axis.edges(),
        1.0,
    )?)?);
    out.extend(checked(taxis("yaxis", "", 1, 0.0, 1.0, &[], 0.0)?)?);
    out.extend(checked(taxis("zaxis", "", 1, 0.0, 1.0, &[], 1.0)?)?);

    put_i16(&mut out, 0);
    put_i16(&mut out, 1000);
    put_f64(&mut out, histogram.entries);
    put_f64(&mut out, in_range_sum(&histogram.contents));
    put_f64(&mut out, in_range_sum(&histogram.sumw2));
    put_f64(&mut out, histogram.tsumwx);
    put_f64(&mut out, histogram.tsumwx2);
    put_f64(&mut out, -1111.0);
    put_f64(&mut out, -1111.0);
    put_f64(&mut out, 0.0);

    out.extend(tarray_d(&[])?);
    out.extend(tarray_d(&histogram.sumw2)?);
    put_string(&mut out, "");
    out.extend(checked(empty_tlist())?);
    put_i32(&mut out, 0);
    put_u8(&mut out, 0);
    put_i32(&mut out, 0);
    put_i32(&mut out, 2);
    Ok(out)
}

fn in_range_sum(values: &[f64]) -> f64 {
    values
        .get(1..values.len().saturating_sub(1))
        .unwrap_or(&[])
        .iter()
        .sum()
}

fn taxis(
    name: &str,
    title: &str,
    bins: usize,
    low: f64,
    high: f64,
    edges: &[f64],
    title_offset: f32,
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    put_u16(&mut out, 10);
    out.extend(checked(tnamed(name, title))?);
    out.extend(checked(tattaxis_v4(title_offset))?);
    put_i32(
        &mut out,
        i32::try_from(bins).map_err(|_| Error::unsupported(name, "too many axis bins"))?,
    );
    put_f64(&mut out, low);
    put_f64(&mut out, high);
    out.extend(tarray_d(edges)?);
    put_i32(&mut out, 0);
    put_i32(&mut out, 0);
    put_u16(&mut out, 0);
    put_u8(&mut out, 0);
    put_string(&mut out, "");
    put_u32(&mut out, 0);
    put_u32(&mut out, 0);
    Ok(out)
}

fn build_tree_object(
    tree_name: &str,
    branches: &[Branch],
    branch_meta: &[BranchMeta],
    baskets: &[BasketInfo],
    entries: usize,
) -> Result<Vec<u8>> {
    let tree_key_len = key_header_len("TTree", tree_name, tree_name);
    let mut tree = Vec::new();
    put_u16(&mut tree, 18);
    tree.extend(checked(tnamed(tree_name, tree_name))?);
    tree.extend(checked(tattline_v1())?);
    tree.extend(checked(tattfill_v1())?);
    tree.extend(checked(tattmarker_v2())?);
    put_i64(&mut tree, entries as i64);
    let total_payload = baskets
        .iter()
        .map(|b| b.uncompressed_len as i64)
        .sum::<i64>();
    let total_basket = baskets.iter().map(|b| b.bytes.len() as i64).sum::<i64>();
    put_i64(&mut tree, total_payload);
    put_i64(&mut tree, total_basket);
    put_i64(&mut tree, 0);
    put_i64(&mut tree, 0);
    put_f64(&mut tree, 1.0);
    put_i32(&mut tree, 0);
    put_i32(&mut tree, 50);
    put_i32(&mut tree, 0);
    put_i32(&mut tree, 0);
    put_i64(&mut tree, 1_000_000_000_000);
    put_i64(&mut tree, 1_000_000_000_000);
    put_i64(&mut tree, 0);
    put_i64(&mut tree, -300000000);
    put_i64(&mut tree, 0);
    put_i64(&mut tree, entries as i64);

    let branches_checked_start = tree.len();
    let mut branch_array = tobjarray_header("", branches.len());
    let mut leaf_refs = Vec::with_capacity(branches.len());
    let mut leaf_count_refs = Vec::with_capacity(branches.len());
    let mut next_object_index = 1_u32;
    for (branch_index, ((branch, meta), basket)) in
        branches.iter().zip(branch_meta).zip(baskets).enumerate()
    {
        let counter_ref = meta
            .counter
            .map(|counter_index| leaf_count_refs[counter_index]);
        let _branch_ref = read_order_object_reference_tag(next_object_index)?;
        next_object_index = next_object_index.saturating_add(1);
        let leaf_count_ref = read_order_object_reference_tag(next_object_index)?;
        next_object_index = next_object_index.saturating_add(1);
        let branch_raw_start = branches_checked_start + 4 + branch_array.len();
        let ctx = BranchBuildContext {
            branch,
            meta,
            basket,
            entries,
            tree_key_len,
            branch_raw_start,
            counter_ref,
            leaf_count_ref,
        };
        let built = build_branch_raw_object(&ctx)?;
        leaf_refs.push(built.leaf_tree_ref);
        leaf_count_refs.push(built.leaf_count_ref);
        debug_assert_eq!(leaf_refs.len(), branch_index + 1);
        debug_assert_eq!(leaf_count_refs.len(), branch_index + 1);
        branch_array.extend(built.raw_object);
    }
    tree.extend(checked(branch_array)?);

    tree.extend(checked(tobjarray_refs("", &leaf_refs))?);

    put_u32(&mut tree, 0);
    put_i32(&mut tree, 0);
    put_i32(&mut tree, 0);
    put_u32(&mut tree, 0);
    put_u32(&mut tree, 0);
    put_u32(&mut tree, 0);
    put_u32(&mut tree, 0);
    checked(tree)
}

struct BuiltBranch {
    raw_object: Vec<u8>,
    leaf_tree_ref: u32,
    leaf_count_ref: u32,
}

struct BranchBuildContext<'a> {
    branch: &'a Branch,
    meta: &'a BranchMeta,
    basket: &'a BasketInfo,
    entries: usize,
    tree_key_len: usize,
    branch_raw_start: usize,
    counter_ref: Option<u32>,
    leaf_count_ref: u32,
}

fn build_branch_raw_object(ctx: &BranchBuildContext<'_>) -> Result<BuiltBranch> {
    let branch_body = build_branch(ctx)?;
    Ok(BuiltBranch {
        raw_object: raw_object("TBranch", branch_body.bytes)?,
        leaf_tree_ref: branch_body.leaf_tree_ref,
        leaf_count_ref: branch_body.leaf_count_ref,
    })
}

struct BuiltBranchBody {
    bytes: Vec<u8>,
    leaf_tree_ref: u32,
    leaf_count_ref: u32,
}

fn build_branch(ctx: &BranchBuildContext<'_>) -> Result<BuiltBranchBody> {
    let mut out = Vec::new();
    let counter_name = ctx.meta.counter.map(|_| leaf_count_name(ctx.branch));
    put_u16(&mut out, 12);
    out.extend(checked(tnamed(
        &ctx.branch.name,
        &ctx.branch
            .data
            .branch_title(&ctx.branch.name, counter_name.as_deref()),
    ))?);
    out.extend(checked(tattfill_v1())?);
    put_i32(&mut out, 0);
    put_i32(&mut out, 32000);
    put_i32(&mut out, ctx.branch.data.entry_offset_len(ctx.entries));
    put_i32(&mut out, 1);
    put_i64(&mut out, ctx.entries as i64);
    put_i32(&mut out, 0);
    put_i32(&mut out, 1);
    put_i32(&mut out, 0);
    put_i64(&mut out, ctx.entries as i64);
    put_i64(&mut out, 0);
    put_i64(&mut out, ctx.basket.uncompressed_len as i64);
    put_i64(&mut out, ctx.basket.bytes.len() as i64);
    out.extend(checked(tobjarray("", Vec::new()))?);
    let leaf_array_checked_start = out.len();
    let mut leaf_array = tobjarray_header("", 1);
    let leaf_raw_start = ctx.branch_raw_start
        + raw_object_prefix_len("TBranch")
        + 4
        + leaf_array_checked_start
        + 4
        + leaf_array.len();
    let leaf_tree_ref = key_framed_byte_reference_tag(ctx.tree_key_len, leaf_raw_start)?;
    leaf_array.extend(raw_object(
        ctx.branch.data.leaf_class(),
        build_leaf(ctx.branch, ctx.meta, ctx.counter_ref)?,
    )?);
    out.extend(checked(leaf_array)?);
    out.extend(checked(tobjarray("", Vec::new()))?);
    put_u8(&mut out, 1);
    put_i32(&mut out, ctx.basket.bytes.len() as i32);
    put_u8(&mut out, 1);
    put_i64(&mut out, 0);
    put_u8(&mut out, 2);
    put_u64(&mut out, ctx.basket.seek);
    put_string(&mut out, "");
    Ok(BuiltBranchBody {
        bytes: out,
        leaf_tree_ref,
        leaf_count_ref: ctx.leaf_count_ref,
    })
}

fn read_order_object_reference_tag(read_order_index: u32) -> Result<u32> {
    read_order_index
        .checked_add(TBUFFER_OBJECT_MAP_OFFSET as u32)
        .ok_or_else(|| Error::unsupported("object reference", "read-order index overflow"))
}

fn key_framed_byte_reference_tag(
    tree_key_len: usize,
    tree_body_raw_object_start: usize,
) -> Result<u32> {
    let map_offset = tree_key_len as u64 + TBUFFER_OBJECT_MAP_OFFSET;
    let local_offset = 4_usize
        .checked_add(tree_body_raw_object_start)
        .ok_or_else(|| Error::unsupported("object reference", "offset overflow"))?;
    byte_reference_tag(map_offset, local_offset)
}

fn byte_reference_tag(map_offset: u64, local_offset: usize) -> Result<u32> {
    let local = u64::try_from(local_offset)
        .map_err(|_| Error::parse(0, "object reference local offset overflows u64"))?;
    let tag = map_offset
        .checked_add(local)
        .ok_or_else(|| Error::parse(0, "object reference tag overflow"))?;
    u32::try_from(tag).map_err(|_| Error::parse(0, "object reference tag overflows u32"))
}

fn leaf_count_name(branch: &Branch) -> String {
    counter_name_for_branch(branch).unwrap_or_default()
}

fn build_leaf(branch: &Branch, meta: &BranchMeta, counter_ref: Option<u32>) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let counter_name = meta.counter.map(|_| leaf_count_name(branch));
    put_u16(&mut out, 1);

    let mut base = Vec::new();
    put_u16(&mut base, 2);
    base.extend(checked(tnamed(
        &branch.name,
        &branch
            .data
            .leaf_title(&branch.name, counter_name.as_deref()),
    ))?);
    put_i32(&mut base, 1);
    put_i32(&mut base, branch.data.element_size());
    put_i32(&mut base, 0);
    put_u8(&mut base, u8::from(meta.is_counter));
    put_u8(&mut base, u8::from(branch.data.is_unsigned()));

    // ROOT stores `TLeaf::fLeafCount` with `TBufferFile::WriteObjectAny`.
    // For an already-written counter leaf, write the uproot-compatible
    // read-order object-map index plus ROOT's object-map offset (2).
    put_u32(&mut base, counter_ref.unwrap_or(0));
    out.extend(checked(base)?);

    branch.data.write_min_max(&mut out);
    Ok(out)
}

fn build_basket(
    branch: &Branch,
    tree_name: &str,
    seek: u64,
    payload: &[u8],
    entries: usize,
) -> Vec<u8> {
    let branch_name = &branch.name;
    let title = format!("{tree_name} basket for {branch_name}");
    let header_len = key_header_len("TBasket", branch_name, &title) + 19;
    let offset_table_len = if branch.data.is_jagged() {
        4 * (branch.data.len() + 2)
    } else {
        0
    };
    let obj_len = payload.len() + offset_table_len;
    let total_size = header_len + obj_len;
    let last = if branch.data.is_jagged() {
        header_len + payload.len()
    } else {
        total_size
    };
    let version = if branch.data.is_jagged() { 3 } else { 2 };
    let nev_buf_size = if branch.data.is_jagged() {
        1000
    } else {
        branch_element_size(payload, entries)
    };
    let spec = TKeySpec {
        total_size: total_size as u32,
        version: 1004,
        uncomp_len: obj_len as u32,
        datime: 0,
        key_len: header_len as i16,
        cycle: 0,
        seek_key: seek,
        seek_pdir: DIRECTORY_OFFSET,
        class_name: "TBasket".to_string(),
        obj_name: branch_name.to_string(),
        obj_title: title,
    };

    let mut out = Vec::new();
    write_key_header(&mut out, &spec);
    put_u16(&mut out, version);
    put_u32(&mut out, 32000);
    put_u32(&mut out, nev_buf_size);
    put_u32(&mut out, entries as u32);
    put_u32(&mut out, last as u32);
    put_i8(&mut out, 0);
    out.extend(payload);
    if let Some(row_lengths) = branch.data.row_lengths() {
        write_jagged_entry_offsets(
            &mut out,
            &row_lengths,
            header_len,
            branch.data.element_size(),
        );
    }
    out
}

fn write_jagged_entry_offsets(
    out: &mut Vec<u8>,
    row_lengths: &[usize],
    key_len: usize,
    element_size: i32,
) {
    put_u32(out, row_lengths.len() as u32 + 1);
    let mut offset = key_len;
    for &row_len in row_lengths {
        put_i32(out, offset as i32);
        offset += row_len * element_size as usize;
    }
    put_i32(out, 0);
}

fn branch_element_size(payload: &[u8], entries: usize) -> u32 {
    payload.len().checked_div(entries).unwrap_or(0) as u32
}

fn build_key(
    class_name: &str,
    obj_name: &str,
    title: &str,
    seek: u64,
    obj: &[u8],
) -> Result<Vec<u8>> {
    let spec = key_spec(class_name, obj_name, title, seek, obj.len(), false)?;
    let mut out = Vec::new();
    write_key_header(&mut out, &spec);
    out.extend(obj);
    Ok(out)
}

fn key_spec(
    class_name: &str,
    obj_name: &str,
    title: &str,
    seek: u64,
    obj_len: usize,
    basket: bool,
) -> Result<TKeySpec> {
    let extra = if basket { 19 } else { 0 };
    let key_len = key_header_len(class_name, obj_name, title) + extra;
    Ok(TKeySpec {
        total_size: u32::try_from(key_len + obj_len)
            .map_err(|_| Error::unsupported("TKey", "object too large"))?,
        version: 1004,
        uncomp_len: u32::try_from(obj_len)
            .map_err(|_| Error::unsupported("TKey", "payload too large"))?,
        datime: 0,
        key_len: i16::try_from(key_len)
            .map_err(|_| Error::unsupported("TKey", "header too large"))?,
        cycle: 1,
        seek_key: seek,
        seek_pdir: DIRECTORY_OFFSET,
        class_name: class_name.to_string(),
        obj_name: obj_name.to_string(),
        obj_title: title.to_string(),
    })
}

fn write_key_header(out: &mut Vec<u8>, spec: &TKeySpec) {
    put_u32(out, spec.total_size);
    put_u16(out, spec.version);
    put_u32(out, spec.uncomp_len);
    put_u32(out, spec.datime);
    put_i16(out, spec.key_len);
    put_i16(out, spec.cycle);
    put_u64(out, spec.seek_key);
    put_u64(out, spec.seek_pdir);
    put_string(out, &spec.class_name);
    put_string(out, &spec.obj_name);
    put_string(out, &spec.obj_title);
}

fn key_header_len(class_name: &str, obj_name: &str, title: &str) -> usize {
    4 + 2
        + 4
        + 4
        + 2
        + 2
        + 8
        + 8
        + root_string_len(class_name)
        + root_string_len(obj_name)
        + root_string_len(title)
}

fn tnamed(name: &str, title: &str) -> Vec<u8> {
    let mut out = Vec::new();
    put_u16(&mut out, 1);
    put_u16(&mut out, 1);
    put_u32(&mut out, 0);
    put_u32(&mut out, 0);
    put_string(&mut out, name);
    put_string(&mut out, title);
    out
}

fn tattline_v1() -> Vec<u8> {
    let mut out = Vec::new();
    put_u16(&mut out, 1);
    put_i16(&mut out, 1);
    put_i16(&mut out, 1);
    put_i16(&mut out, 1);
    out
}

fn tattline_v2() -> Vec<u8> {
    let mut out = Vec::new();
    put_u16(&mut out, 2);
    put_i16(&mut out, 1);
    put_i16(&mut out, 1);
    put_i16(&mut out, 1);
    out
}

fn tattfill_v1() -> Vec<u8> {
    let mut out = Vec::new();
    put_u16(&mut out, 1);
    put_i16(&mut out, 0);
    put_i16(&mut out, 1001);
    out
}

fn tattfill_v2() -> Vec<u8> {
    let mut out = Vec::new();
    put_u16(&mut out, 2);
    put_i16(&mut out, 0);
    put_i16(&mut out, 1001);
    out
}

fn tattmarker_v2() -> Vec<u8> {
    let mut out = Vec::new();
    put_u16(&mut out, 2);
    put_i16(&mut out, 1);
    put_i16(&mut out, 1);
    put_f32(&mut out, 1.0);
    out
}

fn tattaxis_v4(title_offset: f32) -> Vec<u8> {
    let mut out = Vec::new();
    put_u16(&mut out, 4);
    put_i32(&mut out, 510);
    put_i16(&mut out, 1);
    put_i16(&mut out, 1);
    put_i16(&mut out, 42);
    put_f32(&mut out, 0.005);
    put_f32(&mut out, 0.035);
    put_f32(&mut out, 0.03);
    put_f32(&mut out, title_offset);
    put_f32(&mut out, 0.035);
    put_i16(&mut out, 1);
    put_i16(&mut out, 42);
    out
}

fn tarray_f(values: &[f64]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    put_i32(
        &mut out,
        i32::try_from(values.len()).map_err(|_| Error::unsupported("TArrayF", "too large"))?,
    );
    for &value in values {
        put_f32(&mut out, value as f32);
    }
    Ok(out)
}

fn tarray_d(values: &[f64]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    put_i32(
        &mut out,
        i32::try_from(values.len()).map_err(|_| Error::unsupported("TArrayD", "too large"))?,
    );
    for &value in values {
        put_f64(&mut out, value);
    }
    Ok(out)
}

fn tobjarray(name: &str, objects: Vec<Vec<u8>>) -> Vec<u8> {
    let mut out = tobjarray_header(name, objects.len());
    for object in objects {
        out.extend(object);
    }
    out
}

fn tobjarray_header(name: &str, size: usize) -> Vec<u8> {
    let mut out = Vec::new();
    put_u16(&mut out, 3);
    put_u16(&mut out, 1);
    put_u32(&mut out, 0);
    put_u32(&mut out, 0);
    put_c_string(&mut out, name);
    put_i32(&mut out, size as i32);
    put_i32(&mut out, 0);
    out
}

fn tobjarray_refs(name: &str, refs: &[u32]) -> Vec<u8> {
    let mut out = tobjarray_header(name, refs.len());
    for reference in refs {
        put_u32(&mut out, *reference);
    }
    out
}

fn empty_tlist() -> Vec<u8> {
    let mut out = Vec::new();
    put_u16(&mut out, 5);
    put_u16(&mut out, 1);
    put_u32(&mut out, 0);
    put_u32(&mut out, 0);
    put_string(&mut out, "");
    put_i32(&mut out, 0);
    out
}

fn raw_object(class_name: &str, obj: Vec<u8>) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    put_u32(&mut out, 0xffff_ffff);
    put_c_string(&mut out, class_name);
    out.extend(checked(obj)?);
    Ok(out)
}

fn raw_object_prefix_len(class_name: &str) -> usize {
    4 + class_name.len() + 1
}

fn checked(obj: Vec<u8>) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(obj.len() + 4);
    let len = u32::try_from(obj.len())
        .map_err(|_| Error::unsupported("checked ROOT object", "payload too large"))?;
    put_u32(&mut out, 0x4000_0000 | len);
    out.extend(obj);
    Ok(out)
}

fn write_min_max_u8(out: &mut Vec<u8>, values: impl Iterator<Item = u8>) {
    let (min, max) = min_max(values).unwrap_or((0, 0));
    put_u8(out, min);
    put_u8(out, max);
}

fn write_min_max_i8(out: &mut Vec<u8>, values: impl Iterator<Item = i8>) {
    let (min, max) = min_max(values).unwrap_or((0, 0));
    put_i8(out, min);
    put_i8(out, max);
}

fn write_min_max_u16(out: &mut Vec<u8>, values: impl Iterator<Item = u16>) {
    let (min, max) = min_max(values).unwrap_or((0, 0));
    put_u16(out, min);
    put_u16(out, max);
}

fn write_min_max_i16(out: &mut Vec<u8>, values: impl Iterator<Item = i16>) {
    let (min, max) = min_max(values).unwrap_or((0, 0));
    put_i16(out, min);
    put_i16(out, max);
}

fn write_min_max_u32(out: &mut Vec<u8>, values: impl Iterator<Item = u32>) {
    let (min, max) = min_max(values).unwrap_or((0, 0));
    put_u32(out, min);
    put_u32(out, max);
}

fn write_min_max_i32(out: &mut Vec<u8>, values: impl Iterator<Item = i32>) {
    let (min, max) = min_max(values).unwrap_or((0, 0));
    put_i32(out, min);
    put_i32(out, max);
}

fn write_min_max_u64(out: &mut Vec<u8>, values: impl Iterator<Item = u64>) {
    let (min, max) = min_max(values).unwrap_or((0, 0));
    put_u64(out, min);
    put_u64(out, max);
}

fn write_min_max_i64(out: &mut Vec<u8>, values: impl Iterator<Item = i64>) {
    let (min, max) = min_max(values).unwrap_or((0, 0));
    put_i64(out, min);
    put_i64(out, max);
}

fn write_min_max_f32(out: &mut Vec<u8>, values: impl Iterator<Item = f32>) {
    let (min, max) = values.fold((0.0_f32, 0.0_f32), |(min, max), value| {
        (min.min(value), max.max(value))
    });
    put_f32(out, min);
    put_f32(out, max);
}

fn write_min_max_f64(out: &mut Vec<u8>, values: impl Iterator<Item = f64>) {
    let (min, max) = values.fold((0.0_f64, 0.0_f64), |(min, max), value| {
        (min.min(value), max.max(value))
    });
    put_f64(out, min);
    put_f64(out, max);
}

fn write_min_max_bool(out: &mut Vec<u8>, values: impl Iterator<Item = bool>) {
    let values = values.collect::<Vec<_>>();
    put_u8(out, u8::from(values.iter().any(|value| !*value)));
    put_u8(out, u8::from(values.iter().any(|value| *value)));
}

fn min_max<T: Ord + Copy>(mut values: impl Iterator<Item = T>) -> Option<(T, T)> {
    let first = values.next()?;
    Some(values.fold((first, first), |(min, max), value| {
        (min.min(value), max.max(value))
    }))
}

fn put_string(out: &mut Vec<u8>, value: &str) {
    let bytes = value.as_bytes();
    if bytes.len() < 255 {
        put_u8(out, bytes.len() as u8);
    } else {
        put_u8(out, 255);
        put_u32(out, bytes.len() as u32);
    }
    out.extend(bytes);
}

fn root_string_len(value: &str) -> usize {
    if value.len() < 255 {
        1 + value.len()
    } else {
        5 + value.len()
    }
}

fn root_c_string_len(value: &str) -> usize {
    value.len() + 1
}

fn put_c_string(out: &mut Vec<u8>, value: &str) {
    out.extend(value.as_bytes());
    out.push(0);
}

fn put_u8(out: &mut Vec<u8>, value: u8) {
    out.push(value);
}

fn put_i8(out: &mut Vec<u8>, value: i8) {
    out.push(value as u8);
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend(value.to_be_bytes());
}

fn put_i16(out: &mut Vec<u8>, value: i16) {
    out.extend(value.to_be_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend(value.to_be_bytes());
}

fn put_i32(out: &mut Vec<u8>, value: i32) {
    out.extend(value.to_be_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend(value.to_be_bytes());
}

fn put_i64(out: &mut Vec<u8>, value: i64) {
    out.extend(value.to_be_bytes());
}

fn put_f32(out: &mut Vec<u8>, value: f32) {
    out.extend(value.to_bits().to_be_bytes());
}

fn put_f64(out: &mut Vec<u8>, value: f64) {
    out.extend(value.to_bits().to_be_bytes());
}
