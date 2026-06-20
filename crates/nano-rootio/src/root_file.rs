use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::decompress::decompress_root_blocks;
use crate::error::{Error, Result};
use crate::parse::{Cursor, ObjectContext, TBUFFER_OBJECT_MAP_OFFSET};
use crate::tree::{parse_tree, Tree};

const FILE_HEADER_SIZE: usize = 75;
const TDIRECTORY_MAX_SIZE: usize = 42;

#[derive(Debug, Clone)]
pub(crate) struct Source {
    path: Arc<PathBuf>,
}

impl Source {
    fn new(path: PathBuf) -> Self {
        Self {
            path: Arc::new(path),
        }
    }

    pub(crate) fn fetch(&self, offset: u64, len: u64) -> Result<Vec<u8>> {
        let len = usize::try_from(len)
            .map_err(|_| Error::parse(0, format!("read length {len} overflows usize")))?;
        let mut file = File::open(&*self.path)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut out = vec![0; len];
        file.read_exact(&mut out)?;
        Ok(out)
    }
}

#[derive(Debug, Clone)]
struct FileHeader {
    end: u64,
    seek_info: u64,
    nbytes_info: i32,
    seek_dir: u64,
}

#[derive(Debug, Clone)]
struct Directory {
    n_bytes_keys: i32,
    seek_keys: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct TKeyHeader {
    pub(crate) total_size: u32,
    pub(crate) uncompressed_len: u32,
    pub(crate) key_len: i16,
    pub(crate) seek_key: u64,
    pub(crate) class_name: String,
    pub(crate) object_name: String,
}

#[derive(Debug, Clone)]
pub struct FileObject {
    name: String,
    class: String,
}

impl FileObject {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn class(&self) -> &str {
        &self.class
    }
}

#[derive(Debug, Clone)]
struct StreamerInfo {
    name: String,
}

#[derive(Debug, Clone)]
struct FileItem {
    source: Source,
    key: TKeyHeader,
}

#[derive(Debug)]
pub struct RootFile {
    header: FileHeader,
    items: Vec<FileItem>,
    streamer_infos: Vec<StreamerInfo>,
}

impl RootFile {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let source = Source::new(path.as_ref().to_path_buf());
        let header = parse_file_header(&source.fetch(0, FILE_HEADER_SIZE as u64)?)?;
        let directory =
            parse_directory(&source.fetch(header.seek_dir, TDIRECTORY_MAX_SIZE as u64)?)?;
        let key_list_key = parse_tkey(&source.fetch(
            directory.seek_keys,
            u64::try_from(directory.n_bytes_keys).unwrap_or_default(),
        )?)?;
        let items = parse_key_list(&key_list_key.payload)?
            .into_iter()
            .map(|key| FileItem {
                source: source.clone(),
                key,
            })
            .collect::<Vec<_>>();
        let streamer_infos = parse_streamer_infos(&source, &header).unwrap_or_default();
        Ok(Self {
            header,
            items,
            streamer_infos,
        })
    }

    pub fn objects(&self) -> Vec<FileObject> {
        self.items
            .iter()
            .map(|item| FileObject {
                name: item.key.object_name.clone(),
                class: item.key.class_name.clone(),
            })
            .collect()
    }

    pub fn streamer_info_names(&self) -> Vec<String> {
        self.streamer_infos
            .iter()
            .map(|info| info.name.clone())
            .collect()
    }

    pub fn tree(&self, name: &str) -> Result<Tree> {
        let item = self
            .items
            .iter()
            .find(|item| item.key.object_name == name && item.key.class_name == "TTree")
            .ok_or_else(|| Error::MissingTree(name.to_string()))?;
        item.as_tree()
    }

    pub fn file_size(&self) -> u64 {
        self.header.end
    }
}

impl FileItem {
    fn payload(&self) -> Result<Vec<u8>> {
        let payload_offset = self.key.seek_key + self.key.key_len as u64;
        let payload_len = self.key.total_size - self.key.key_len as u32;
        let payload = self.source.fetch(payload_offset, payload_len as u64)?;
        if self.key.total_size < self.key.uncompressed_len {
            decompress_root_blocks(&payload)
        } else {
            Ok(payload)
        }
    }

    fn as_tree(&self) -> Result<Tree> {
        let payload = self.payload()?;
        let mut cur = Cursor::new(&payload);
        let mut tree_payload = cur.checked_sub()?;
        let ctx = ObjectContext::new(
            &payload,
            self.key.key_len as u64 + TBUFFER_OBJECT_MAP_OFFSET,
        );
        parse_tree(&mut tree_payload, &ctx, self.source.clone())
    }
}

pub(crate) struct TKey {
    header: TKeyHeader,
    payload: Vec<u8>,
}

pub(crate) fn parse_tkey_header(cur: &mut Cursor<'_>) -> Result<TKeyHeader> {
    let total_size = cur.u32()?;
    let version = cur.u16()?;
    let uncompressed_len = cur.u32()?;
    let _datime = cur.u32()?;
    let key_len = cur.i16()?;
    let _cycle = cur.i16()?;
    let seek_key = cur.seek_pointer(version)?;
    let _seek_parent_dir = cur.seek_pointer(version)?;
    let class_name = cur.string()?;
    let object_name = cur.string()?;
    let _object_title = cur.string()?;
    Ok(TKeyHeader {
        total_size,
        uncompressed_len,
        key_len,
        seek_key,
        class_name,
        object_name,
    })
}

pub(crate) fn parse_tkey(bytes: &[u8]) -> Result<TKey> {
    let mut cur = Cursor::new(bytes);
    let header = parse_tkey_header(&mut cur)?;
    let payload_len = header
        .total_size
        .checked_sub(header.key_len as u32)
        .ok_or_else(|| Error::parse(cur.position(), "TKey total size smaller than key length"))?;
    let payload = cur.take(payload_len as usize)?.to_vec();
    let payload = if header.uncompressed_len as usize > payload.len() {
        decompress_root_blocks(&payload)?
    } else {
        payload
    };
    Ok(TKey { header, payload })
}

fn parse_file_header(bytes: &[u8]) -> Result<FileHeader> {
    let mut cur = Cursor::new(bytes);
    if cur.take(4)? != b"root" {
        return Err(Error::parse(0, "missing ROOT magic"));
    }
    let version = cur.i32()?;
    let is_64_bit = version > 1_000_000;
    let versioned = |cur: &mut Cursor<'_>| {
        if is_64_bit {
            cur.u64()
        } else {
            Ok(cur.u32()? as u64)
        }
    };
    let begin = cur.i32()?;
    let end = versioned(&mut cur)?;
    let _seek_free = versioned(&mut cur)?;
    let _nbytes_free = cur.i32()?;
    let _n_entries_free = cur.i32()?;
    let n_bytes_name = cur.i32()?;
    let _pointer_size = cur.u8()?;
    let _compression = cur.i32()?;
    let seek_info = versioned(&mut cur)?;
    let nbytes_info = cur.i32()?;
    let _uuid_version = cur.u16()?;
    let _uuid_bytes = cur.take(16)?;
    let seek_dir = (begin + n_bytes_name) as u64;
    Ok(FileHeader {
        end,
        seek_info,
        nbytes_info,
        seek_dir,
    })
}

fn parse_directory(bytes: &[u8]) -> Result<Directory> {
    let mut cur = Cursor::new(bytes);
    let version = cur.i16()?;
    let _ctime = cur.u32()?;
    let _mtime = cur.u32()?;
    let n_bytes_keys = cur.i32()?;
    let _n_bytes_name = cur.i32()?;
    let _seek_dir = cur.versioned_pointer(version)?;
    let _seek_parent = cur.versioned_pointer(version)?;
    let seek_keys = cur.versioned_pointer(version)?;
    Ok(Directory {
        n_bytes_keys,
        seek_keys,
    })
}

fn parse_key_list(bytes: &[u8]) -> Result<Vec<TKeyHeader>> {
    let mut cur = Cursor::new(bytes);
    let len = cur.u32()?;
    let mut out = Vec::with_capacity(len as usize);
    for _ in 0..len {
        out.push(parse_tkey_header(&mut cur)?);
    }
    Ok(out)
}

fn parse_streamer_infos(source: &Source, header: &FileHeader) -> Result<Vec<StreamerInfo>> {
    if header.seek_info == 0 || header.nbytes_info <= 0 {
        return Ok(Vec::new());
    }
    let info_key = parse_tkey(&source.fetch(
        header.seek_info,
        u64::try_from(header.nbytes_info + 4).unwrap_or_default(),
    )?)?;
    let ctx = ObjectContext::new(
        &info_key.payload,
        info_key.header.key_len as u64 + TBUFFER_OBJECT_MAP_OFFSET,
    );
    let mut cur = Cursor::new(&info_key.payload);
    let mut list = cur.checked_sub()?;
    let _version = list.u16()?;
    crate::parse::parse_tobject(&mut list)?;
    let _name = list.string()?;
    let len = list.i32()?;
    let mut infos = Vec::new();
    for _ in 0..len.max(0) {
        let mut obj = list.checked_sub()?;
        let raw = crate::parse::read_raw(&mut obj, &ctx)?;
        let _option = list.string()?;
        infos.push(StreamerInfo {
            name: parse_streamer_name(raw.payload).unwrap_or_else(|| raw.class_name.to_string()),
        });
    }
    Ok(infos)
}

fn parse_streamer_name(payload: &[u8]) -> Option<String> {
    let mut cur = Cursor::new(payload);
    let _version = cur.u16().ok()?;
    let mut named = cur.checked_sub().ok()?;
    crate::parse::parse_tnamed(&mut named)
        .ok()
        .map(|named| named.name)
}
