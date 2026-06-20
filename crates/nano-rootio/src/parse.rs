use std::convert::TryFrom;

use crate::error::{Error, Result};

const BYTE_COUNT_MASK: u32 = 0x4000_0000;
const CLASS_MASK: u32 = 0x8000_0000;
const NEW_CLASSTAG: u32 = 0xFFFF_FFFF;
const IS_REFERENCED: u32 = 1 << 4;
pub(crate) const TBUFFER_OBJECT_MAP_OFFSET: u64 = 2;

#[derive(Clone)]
pub(crate) struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub(crate) fn at(data: &'a [u8], pos: usize) -> Result<Self> {
        if pos > data.len() {
            return Err(Error::parse(pos, "cursor start beyond input"));
        }
        Ok(Self { data, pos })
    }

    pub(crate) fn position(&self) -> usize {
        self.pos
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub(crate) fn rest(&mut self) -> &'a [u8] {
        let out = &self.data[self.pos..];
        self.pos = self.data.len();
        out
    }

    pub(crate) fn peek_u32(&self) -> Result<u32> {
        let mut tmp = self.clone();
        tmp.u32()
    }

    pub(crate) fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| Error::parse(self.pos, "cursor overflow"))?;
        if end > self.data.len() {
            return Err(Error::parse(
                self.pos,
                format!("need {len} bytes, only {} remain", self.remaining()),
            ));
        }
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    pub(crate) fn sub(&mut self, len: usize) -> Result<Cursor<'a>> {
        Ok(Cursor::new(self.take(len)?))
    }

    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn i8(&mut self) -> Result<i8> {
        Ok(self.u8()? as i8)
    }

    pub(crate) fn bool(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(Error::parse(
                self.pos.saturating_sub(1),
                format!("invalid ROOT bool byte {other}"),
            )),
        }
    }

    pub(crate) fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }

    pub(crate) fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }

    pub(crate) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub(crate) fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub(crate) fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }

    pub(crate) fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }

    pub(crate) fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }

    pub(crate) fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.u64()?))
    }

    pub(crate) fn string(&mut self) -> Result<String> {
        let len = match self.u8()? {
            255 => usize::try_from(self.u32()?).unwrap(),
            val => val as usize,
        };
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|err| Error::parse(self.pos.saturating_sub(len), err.to_string()))
    }

    pub(crate) fn c_string(&mut self) -> Result<&'a str> {
        let start = self.pos;
        let rel = self.data[start..]
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| Error::parse(start, "unterminated C string"))?;
        let bytes = &self.data[start..start + rel];
        self.pos = start + rel + 1;
        std::str::from_utf8(bytes).map_err(|err| Error::parse(start, err.to_string()))
    }

    pub(crate) fn checked_byte_count(&mut self) -> Result<usize> {
        let offset = self.pos;
        let raw = self.u32()?;
        if raw & BYTE_COUNT_MASK == 0 || raw == NEW_CLASSTAG {
            return Err(Error::parse(offset, "expected ROOT checked byte count"));
        }
        Ok((raw & !BYTE_COUNT_MASK) as usize)
    }

    pub(crate) fn checked_sub(&mut self) -> Result<Cursor<'a>> {
        let len = self.checked_byte_count()?;
        self.sub(len)
    }

    pub(crate) fn versioned_pointer(&mut self, version: i16) -> Result<u64> {
        if version > 1000 {
            self.u64()
        } else {
            Ok(self.i32()? as u64)
        }
    }

    pub(crate) fn seek_pointer(&mut self, version: u16) -> Result<u64> {
        if version > 1000 {
            self.u64()
        } else {
            Ok(self.u32()? as u64)
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TNamed {
    pub(crate) name: String,
    pub(crate) _title: String,
}

pub(crate) fn parse_tobject(cur: &mut Cursor<'_>) -> Result<()> {
    let _version = cur.u16()?;
    let _id = cur.u32()?;
    let bits = cur.u32()?;
    if bits & IS_REFERENCED != 0 {
        let _process_id = cur.u16()?;
    }
    Ok(())
}

pub(crate) fn parse_tnamed(cur: &mut Cursor<'_>) -> Result<TNamed> {
    let _version = cur.u16()?;
    parse_tobject(cur)?;
    let name = cur.string()?;
    let _title = cur.string()?;
    Ok(TNamed { name, _title })
}

#[derive(Debug, Clone)]
enum ClassInfo<'a> {
    New(&'a str),
    Exists(u32),
    References(u32),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RawObject<'a> {
    pub(crate) class_name: &'a str,
    pub(crate) payload: &'a [u8],
}

/// Parser context for ROOT object references.
///
/// ROOT's `TBufferFile::ReadObjectAny` serializes objects with compact class
/// tags and sometimes replaces an object occurrence with an absolute back
/// reference into the same buffer.  The `map_offset` maps positions inside
/// `bytes` to those absolute tags.  Readers and the future writer must agree on
/// this scheme: a new class stores a C-string class name followed by a checked
/// object payload, an existing class stores the prior class tag plus a payload,
/// and an object reference stores only the prior object tag.
pub(crate) struct ObjectContext<'a> {
    bytes: &'a [u8],
    map_offset: u64,
}

impl<'a> ObjectContext<'a> {
    pub(crate) fn new(bytes: &'a [u8], map_offset: u64) -> Self {
        Self { bytes, map_offset }
    }

    fn local_offset(&self, absolute: u32) -> Result<usize> {
        let absolute = absolute as u64;
        if absolute == 0 {
            return Ok(0);
        }
        let local = absolute.checked_sub(self.map_offset).ok_or_else(|| {
            Error::parse(
                0,
                format!(
                    "object reference {absolute} precedes map offset {}",
                    self.map_offset
                ),
            )
        })?;
        usize::try_from(local).map_err(|_| Error::parse(0, "object reference overflows usize"))
    }

    fn raw_at(&self, absolute: u32) -> Result<RawObject<'a>> {
        if absolute == 0 {
            return Ok(RawObject {
                class_name: "",
                payload: &self.bytes[..0],
            });
        }
        let pos = self.local_offset(absolute)?;
        let mut cur = Cursor::at(self.bytes, pos)?;
        read_raw(&mut cur, self)
    }

    fn class_name_at(&self, absolute: u32) -> Result<&'a str> {
        self.raw_at(absolute).map(|raw| raw.class_name)
    }
}

/// Return the absolute object-reference tag consumed by `read_raw`.
///
/// ROOT's object pointer references are not file offsets. They are offsets into
/// the current `TBufferFile` payload plus the buffer's object-map offset. The
/// reader subtracts `map_offset` before reparsing the referenced raw object; the
/// writer uses this helper to emit the inverse tag.
pub(crate) fn object_reference_tag(map_offset: u64, local_offset: usize) -> Result<u32> {
    let local = u64::try_from(local_offset)
        .map_err(|_| Error::parse(0, "object reference local offset overflows u64"))?;
    let tag = map_offset
        .checked_add(local)
        .ok_or_else(|| Error::parse(0, "object reference tag overflow"))?;
    u32::try_from(tag).map_err(|_| Error::parse(0, "object reference tag overflows u32"))
}

fn read_class_info<'a>(cur: &mut Cursor<'a>) -> Result<ClassInfo<'a>> {
    let first = cur.u32()?;
    let tag = if first & BYTE_COUNT_MASK != 0 && first != NEW_CLASSTAG {
        cur.u32()?
    } else {
        first
    };
    if tag == NEW_CLASSTAG {
        return Ok(ClassInfo::New(cur.c_string()?));
    }
    if tag & CLASS_MASK != 0 {
        Ok(ClassInfo::Exists(tag & !CLASS_MASK))
    } else {
        Ok(ClassInfo::References(tag))
    }
}

pub(crate) fn read_raw<'a>(cur: &mut Cursor<'a>, ctx: &ObjectContext<'a>) -> Result<RawObject<'a>> {
    match read_class_info(cur)? {
        ClassInfo::New(class_name) => {
            let payload = cur.checked_sub()?.rest();
            Ok(RawObject {
                class_name,
                payload,
            })
        }
        ClassInfo::Exists(tag) => {
            let class_name = ctx.class_name_at(tag)?;
            let payload = cur.checked_sub()?.rest();
            Ok(RawObject {
                class_name,
                payload,
            })
        }
        ClassInfo::References(tag) => ctx.raw_at(tag),
    }
}

pub(crate) fn read_tobjarray<'a, T, F>(
    cur: &mut Cursor<'a>,
    ctx: &ObjectContext<'a>,
    mut parse: F,
) -> Result<Vec<T>>
where
    F: FnMut(RawObject<'a>, &ObjectContext<'a>) -> Result<T>,
{
    let _version = cur.u16()?;
    parse_tobject(cur)?;
    let _name = cur.c_string()?;
    let size = cur.i32()?;
    let _lower_bound = cur.i32()?;
    if size < 0 {
        return Err(Error::parse(cur.position(), "negative TObjArray size"));
    }
    let mut out = Vec::with_capacity(size as usize);
    for _ in 0..size {
        let raw = read_raw(cur, ctx)?;
        out.push(parse(raw, ctx)?);
    }
    Ok(out)
}

pub(crate) fn skip_tiofeatures(cur: &mut Cursor<'_>) -> Result<()> {
    let _ = cur.checked_sub()?;
    Ok(())
}

pub(crate) fn maybe_raw_buffer<'a>(
    cur: &mut Cursor<'a>,
    ctx: &ObjectContext<'a>,
) -> Result<Option<Vec<u8>>> {
    if cur.peek_u32()? == 0 {
        let _ = cur.u32()?;
        Ok(None)
    } else {
        Ok(Some(read_raw(cur, ctx)?.payload.to_vec()))
    }
}
