use std::convert::TryFrom;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use crate::{Result, RootError};

const FILE_BEGIN: u32 = 100;
const DIRECTORY_OFFSET: u64 = FILE_BEGIN as u64;
const DIRECTORY_SIZE: u64 = 30;
const TREE_OFFSET: u64 = DIRECTORY_OFFSET + DIRECTORY_SIZE;

#[derive(Debug, Clone)]
pub struct Branch {
    name: String,
    data: BranchData,
}

#[derive(Debug, Clone)]
pub enum BranchData {
    F32(Vec<f32>),
    VecF32(Vec<Vec<f32>>),
    I32(Vec<i32>),
    U32(Vec<u32>),
    U64(Vec<u64>),
    Bool(Vec<bool>),
}

impl Branch {
    pub fn f32(name: impl Into<String>, values: Vec<f32>) -> Self {
        Self {
            name: name.into(),
            data: BranchData::F32(values),
        }
    }

    pub fn vec_f32(name: impl Into<String>, values: Vec<Vec<f32>>) -> Self {
        Self {
            name: name.into(),
            data: BranchData::VecF32(values),
        }
    }

    pub fn i32(name: impl Into<String>, values: Vec<i32>) -> Self {
        Self {
            name: name.into(),
            data: BranchData::I32(values),
        }
    }

    pub fn u32(name: impl Into<String>, values: Vec<u32>) -> Self {
        Self {
            name: name.into(),
            data: BranchData::U32(values),
        }
    }

    pub fn u64(name: impl Into<String>, values: Vec<u64>) -> Self {
        Self {
            name: name.into(),
            data: BranchData::U64(values),
        }
    }

    pub fn bool(name: impl Into<String>, values: Vec<bool>) -> Self {
        Self {
            name: name.into(),
            data: BranchData::Bool(values),
        }
    }
}

impl BranchData {
    fn len(&self) -> usize {
        match self {
            Self::F32(v) => v.len(),
            Self::VecF32(v) => v.len(),
            Self::I32(v) => v.len(),
            Self::U32(v) => v.len(),
            Self::U64(v) => v.len(),
            Self::Bool(v) => v.len(),
        }
    }

    fn payload(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Self::F32(values) => {
                for value in values {
                    put_u32(&mut out, value.to_bits());
                }
            }
            Self::VecF32(rows) => {
                for row in rows {
                    for value in row {
                        put_u32(&mut out, value.to_bits());
                    }
                }
            }
            Self::I32(values) => {
                for value in values {
                    put_i32(&mut out, *value);
                }
            }
            Self::U32(values) => {
                for value in values {
                    put_u32(&mut out, *value);
                }
            }
            Self::U64(values) => {
                for value in values {
                    put_u64(&mut out, *value);
                }
            }
            Self::Bool(values) => {
                for value in values {
                    put_u8(&mut out, u8::from(*value));
                }
            }
        }
        out
    }

    fn leaf_class(&self) -> &'static str {
        match self {
            Self::F32(_) | Self::VecF32(_) => "TLeafF",
            Self::I32(_) | Self::U32(_) => "TLeafI",
            Self::U64(_) => "TLeafL",
            Self::Bool(_) => "TLeafO",
        }
    }

    fn leaf_title(&self, name: &str) -> String {
        let code = match self {
            Self::F32(_) | Self::VecF32(_) => "F",
            Self::I32(_) => "I",
            Self::U32(_) => "i",
            Self::U64(_) => "l",
            Self::Bool(_) => "O",
        };
        format!("{name}/{code}")
    }

    fn element_size(&self) -> i32 {
        match self {
            Self::F32(_) | Self::VecF32(_) | Self::I32(_) | Self::U32(_) => 4,
            Self::U64(_) => 8,
            Self::Bool(_) => 1,
        }
    }

    fn is_unsigned(&self) -> bool {
        matches!(self, Self::U32(_) | Self::U64(_))
    }

    fn write_min_max(&self, out: &mut Vec<u8>) {
        match self {
            Self::F32(values) => {
                let (min, max) = values
                    .iter()
                    .copied()
                    .fold((0.0_f32, 0.0_f32), |(min, max), value| {
                        (min.min(value), max.max(value))
                    });
                put_u32(out, min.to_bits());
                put_u32(out, max.to_bits());
            }
            Self::VecF32(rows) => {
                let (min, max) = rows
                    .iter()
                    .flatten()
                    .copied()
                    .fold((0.0_f32, 0.0_f32), |(min, max), value| {
                        (min.min(value), max.max(value))
                    });
                put_u32(out, min.to_bits());
                put_u32(out, max.to_bits());
            }
            Self::I32(values) => {
                put_i32(out, values.iter().copied().min().unwrap_or(0));
                put_i32(out, values.iter().copied().max().unwrap_or(0));
            }
            Self::U32(values) => {
                put_i32(
                    out,
                    i32::try_from(values.iter().copied().min().unwrap_or(0)).unwrap_or(0),
                );
                put_i32(
                    out,
                    i32::try_from(values.iter().copied().max().unwrap_or(0)).unwrap_or(0),
                );
            }
            Self::U64(values) => {
                put_i64(
                    out,
                    i64::try_from(values.iter().copied().min().unwrap_or(0)).unwrap_or(0),
                );
                put_i64(
                    out,
                    i64::try_from(values.iter().copied().max().unwrap_or(0)).unwrap_or(0),
                );
            }
            Self::Bool(values) => {
                put_u8(out, u8::from(values.iter().any(|value| !*value)));
                put_u8(out, u8::from(values.iter().any(|value| *value)));
            }
        }
    }
}

#[derive(Debug, Clone)]
struct BasketInfo {
    bytes: Vec<u8>,
    payload_len: usize,
    seek: u64,
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

/// Write an uncompressed ROOT file containing one `TTree`.
///
/// This intentionally covers the narrow NanoAOD bootstrap subset used by the
/// in-tree reader: fixed-size scalar leaves, flattened `Vec<f32>` payloads,
/// and one on-disk basket per branch.
pub fn write_tree<P: AsRef<Path>>(
    path: P,
    tree_name: &str,
    branches: &[Branch],
) -> Result<()> {
    if branches.is_empty() {
        return Err(RootError::other("cannot write a TTree without branches"));
    }

    let entries = branches[0].data.len();
    for branch in branches {
        if branch.name.is_empty() {
            return Err(RootError::other("branch names must not be empty"));
        }
        if branch.data.len() != entries {
            return Err(RootError::other(format!(
                "branch `{}` has {} entries, expected {}",
                branch.name,
                branch.data.len(),
                entries
            )));
        }
    }

    let mut baskets: Vec<BasketInfo> = branches
        .iter()
        .map(|branch| {
            let payload = branch.data.payload();
            let bytes = build_basket(&branch.name, tree_name, 0, &payload, entries);
            BasketInfo {
                bytes,
                payload_len: payload.len(),
                seek: 0,
            }
        })
        .collect();

    let provisional_tree_obj = build_tree_object(tree_name, branches, &baskets, entries);
    let provisional_tree_key = build_key(
        "TTree",
        tree_name,
        tree_name,
        TREE_OFFSET,
        &provisional_tree_obj,
    );
    let mut next_seek = TREE_OFFSET + provisional_tree_key.len() as u64;

    for (branch, basket) in branches.iter().zip(baskets.iter_mut()) {
        basket.seek = next_seek;
        basket.bytes = build_basket(
            &branch.name,
            tree_name,
            basket.seek,
            &branch.data.payload(),
            entries,
        );
        next_seek += basket.bytes.len() as u64;
    }

    let tree_obj = build_tree_object(tree_name, branches, &baskets, entries);
    let tree_key = build_key("TTree", tree_name, tree_name, TREE_OFFSET, &tree_obj);
    let tree_key_header = key_spec(
        "TTree",
        tree_name,
        tree_name,
        TREE_OFFSET,
        tree_obj.len(),
        false,
    );

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
    );

    let streamer_info_offset = key_list_offset + key_list_key.len() as u64;
    let streamer_info_obj = checked(empty_tlist());
    let streamer_info_key = build_key(
        "TList",
        "StreamerInfo",
        "Doubly linked list",
        streamer_info_offset,
        &streamer_info_obj,
    );

    let file_end = streamer_info_offset + streamer_info_key.len() as u64 + 4;
    let mut file_bytes = vec![0; FILE_BEGIN as usize];
    write_file_header(
        &mut file_bytes[..75],
        u32::try_from(file_end)?,
        u32::try_from(streamer_info_offset)?,
        u32::try_from(streamer_info_key.len())?,
    );
    file_bytes.extend(build_directory(
        u32::try_from(key_list_offset)?,
        u32::try_from(key_list_key.len())?,
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

fn write_file_header(out: &mut [u8], end: u32, seek_info: u32, nbytes_info: u32) {
    let mut bytes = Vec::with_capacity(75);
    bytes.extend(b"root");
    put_i32(&mut bytes, 62400);
    put_i32(&mut bytes, FILE_BEGIN as i32);
    put_u32(&mut bytes, end);
    put_u32(&mut bytes, 0);
    put_i32(&mut bytes, 0);
    put_i32(&mut bytes, 0);
    put_i32(&mut bytes, 0);
    put_u8(&mut bytes, 4);
    put_i32(&mut bytes, 0);
    put_u32(&mut bytes, seek_info);
    put_i32(&mut bytes, nbytes_info as i32);
    put_u16(&mut bytes, 1);
    bytes.extend([0; 16]);
    out[..bytes.len()].copy_from_slice(&bytes);
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

fn build_tree_object(
    tree_name: &str,
    branches: &[Branch],
    baskets: &[BasketInfo],
    entries: usize,
) -> Vec<u8> {
    let mut tree = Vec::new();
    put_u16(&mut tree, 18);
    tree.extend(checked(tnamed(tree_name, tree_name)));
    tree.extend(checked(vec![0]));
    tree.extend(checked(vec![0]));
    tree.extend(checked(vec![0]));
    put_i64(&mut tree, entries as i64);
    let total_payload = baskets.iter().map(|b| b.payload_len as i64).sum::<i64>();
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

    let branch_objs = branches
        .iter()
        .zip(baskets)
        .map(|(branch, basket)| raw_object("TBranch", build_branch(branch, basket, entries)))
        .collect();
    tree.extend(checked(tobjarray("", branch_objs)));

    let leaf_objs = branches
        .iter()
        .map(|branch| raw_object(branch.data.leaf_class(), build_leaf(branch)))
        .collect();
    tree.extend(checked(tobjarray("", leaf_objs)));

    put_u32(&mut tree, 0);
    put_i32(&mut tree, 0);
    put_i32(&mut tree, 0);
    put_u32(&mut tree, 0);
    put_u32(&mut tree, 0);
    put_u32(&mut tree, 0);
    put_u32(&mut tree, 0);
    checked(tree)
}

fn build_branch(branch: &Branch, basket: &BasketInfo, entries: usize) -> Vec<u8> {
    let mut out = Vec::new();
    put_u16(&mut out, 12);
    out.extend(checked(tnamed(
        &branch.name,
        &branch.data.leaf_title(&branch.name),
    )));
    out.extend(checked(vec![0]));
    put_i32(&mut out, 0);
    put_i32(&mut out, 32000);
    put_i32(&mut out, 0);
    put_i32(&mut out, 1);
    put_i64(&mut out, entries as i64);
    put_i32(&mut out, 0);
    put_i32(&mut out, 1);
    put_i32(&mut out, 0);
    put_i64(&mut out, entries as i64);
    put_i64(&mut out, 0);
    put_i64(&mut out, basket.payload_len as i64);
    put_i64(&mut out, basket.bytes.len() as i64);
    out.extend(checked(tobjarray("", Vec::new())));
    out.extend(checked(tobjarray(
        "",
        vec![raw_object(branch.data.leaf_class(), build_leaf(branch))],
    )));
    out.extend(checked(tobjarray("", Vec::new())));
    put_u8(&mut out, 1);
    put_i32(&mut out, basket.bytes.len() as i32);
    put_u8(&mut out, 1);
    put_i64(&mut out, 0);
    put_u8(&mut out, 1);
    put_u64(&mut out, basket.seek);
    put_string(&mut out, "");
    out
}

fn build_leaf(branch: &Branch) -> Vec<u8> {
    let mut out = Vec::new();
    put_u16(&mut out, 1);

    let mut base = Vec::new();
    put_u16(&mut base, 2);
    base.extend(checked(tnamed(
        &branch.name,
        &branch.data.leaf_title(&branch.name),
    )));
    put_i32(&mut base, 1);
    put_i32(&mut base, branch.data.element_size());
    put_i32(&mut base, 0);
    put_u8(&mut base, 0);
    put_u8(&mut base, u8::from(branch.data.is_unsigned()));
    put_u32(&mut base, 0);
    out.extend(checked(base));

    branch.data.write_min_max(&mut out);
    out
}

fn build_basket(
    branch_name: &str,
    tree_name: &str,
    seek: u64,
    payload: &[u8],
    entries: usize,
) -> Vec<u8> {
    let title = format!("{tree_name} basket for {branch_name}");
    let header_len = key_header_len("TBasket", branch_name, &title) + 19;
    let total_size = header_len + payload.len();
    let spec = TKeySpec {
        total_size: total_size as u32,
        version: 1004,
        uncomp_len: payload.len() as u32,
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
    put_u16(&mut out, 2);
    put_u32(&mut out, 32000);
    put_u32(&mut out, branch_element_size(payload, entries));
    put_u32(&mut out, entries as u32);
    put_u32(&mut out, total_size as u32);
    put_i8(&mut out, 0);
    out.extend(payload);
    out
}

fn branch_element_size(payload: &[u8], entries: usize) -> u32 {
    if entries == 0 {
        0
    } else {
        (payload.len() / entries) as u32
    }
}

fn build_key(class_name: &str, obj_name: &str, title: &str, seek: u64, obj: &[u8]) -> Vec<u8> {
    let spec = key_spec(class_name, obj_name, title, seek, obj.len(), false);
    let mut out = Vec::new();
    write_key_header(&mut out, &spec);
    out.extend(obj);
    out
}

fn key_spec(
    class_name: &str,
    obj_name: &str,
    title: &str,
    seek: u64,
    obj_len: usize,
    basket: bool,
) -> TKeySpec {
    let extra = if basket { 19 } else { 0 };
    let key_len = key_header_len(class_name, obj_name, title) + extra;
    TKeySpec {
        total_size: (key_len + obj_len) as u32,
        version: 1004,
        uncomp_len: obj_len as u32,
        datime: 0,
        key_len: key_len as i16,
        cycle: 1,
        seek_key: seek,
        seek_pdir: DIRECTORY_OFFSET,
        class_name: class_name.to_string(),
        obj_name: obj_name.to_string(),
        obj_title: title.to_string(),
    }
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

fn tobjarray(name: &str, objects: Vec<Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    put_u16(&mut out, 3);
    put_u16(&mut out, 1);
    put_u32(&mut out, 0);
    put_u32(&mut out, 0);
    put_c_string(&mut out, name);
    put_i32(&mut out, objects.len() as i32);
    put_i32(&mut out, 0);
    for object in objects {
        out.extend(object);
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

fn raw_object(class_name: &str, obj: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::new();
    put_u32(&mut out, 0xffff_ffff);
    put_c_string(&mut out, class_name);
    out.extend(checked(obj));
    out
}

fn checked(obj: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(obj.len() + 4);
    put_u32(&mut out, 0x4000_0000 | obj.len() as u32);
    out.extend(obj);
    out
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

fn put_f64(out: &mut Vec<u8>, value: f64) {
    out.extend(value.to_bits().to_be_bytes());
}
