use nom::multi::length_value;

use crate::core::{checked_byte_count, decompress, Context, Source, TKeyHeader};
use crate::tree_reader::{ttree, Tree};
use crate::{Result, RootError};

/// Describes a single item within this file (e.g. a `Tree`)
#[derive(Debug)]
pub struct FileItem {
    source: Source,
    tkey_hdr: TKeyHeader,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Axis1D {
    pub nbins: i32,
    pub xmin: f64,
    pub xmax: f64,
    pub edges: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Th1F {
    pub name: String,
    pub title: String,
    pub axis: Axis1D,
    pub contents: Vec<f32>,
    pub entries: f64,
    pub tsumw: f64,
    pub tsumw2: f64,
    pub tsumwx: f64,
    pub tsumwx2: f64,
    pub sumw2: Vec<f64>,
}

impl FileItem {
    /// New file item from the information in a TKeyHeader and the associated file
    pub(crate) fn new(tkey_hdr: &TKeyHeader, source: Source) -> FileItem {
        FileItem {
            source,
            tkey_hdr: tkey_hdr.to_owned(),
        }
    }

    /// Information about this file item in Human readable form
    pub fn verbose_info(&self) -> String {
        format!("{:#?}", self.tkey_hdr)
    }
    pub fn name(&self) -> String {
        format!(
            "`{}` of type `{}`",
            self.tkey_hdr.obj_name, self.tkey_hdr.class_name
        )
    }

    async fn get_buffer(&self) -> Result<Vec<u8>> {
        let start = self.tkey_hdr.seek_key + self.tkey_hdr.key_len as u64;
        let len = self.tkey_hdr.total_size - self.tkey_hdr.key_len as u32;
        let comp_buf = self.source.fetch(start, len as u64).await?;

        let buf = if self.tkey_hdr.total_size < self.tkey_hdr.uncomp_len {
            // Decompress the read buffer; buf is Vec<u8>
            let (_, buf) = decompress(comp_buf.as_slice())
                .map_err(|err| RootError::parse(format!("decompression parser failed: {err:?}")))?;
            buf
        } else {
            comp_buf
        };
        Ok(buf)
    }

    pub(crate) async fn get_context<'s>(&self) -> Result<Context> {
        let buffer = self.get_buffer().await?;
        let k_map_offset = 2;
        Ok(Context {
            source: self.source.clone(),
            offset: (self.tkey_hdr.key_len + k_map_offset) as u64,
            s: buffer,
        })
    }

    /// Parse this `FileItem` as a `Tree`
    pub async fn as_tree(&self) -> Result<Tree> {
        let ctx = self.get_context().await?;
        let buf = ctx.s.as_slice();

        let res = length_value(checked_byte_count, |i| ttree(i, &ctx))(buf);
        match res {
            Ok((_, obj)) => Ok(obj),
            Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => {
                Err(RootError::parse(format!("Supplied parser failed! {e:?}")))
            }
            _ => panic!(),
        }
    }

    /// Parse this `FileItem` as a minimal `TH1F` oracle object.
    pub async fn as_th1f(&self) -> Result<Th1F> {
        if self.tkey_hdr.class_name != "TH1F" {
            return Err(RootError::parse(format!(
                "expected TH1F key, got {}",
                self.tkey_hdr.class_name
            )));
        }
        let buffer = self.get_buffer().await?;
        parse_th1f(&buffer)
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| RootError::parse("cursor overflow"))?;
        if end > self.bytes.len() {
            return Err(RootError::parse(format!(
                "need {len} bytes, only {} remain",
                self.remaining()
            )));
        }
        let out = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    fn checked_sub(&mut self) -> Result<Cursor<'a>> {
        let raw = self.u32()?;
        if raw & 0x4000_0000 == 0 {
            return Err(RootError::parse("expected ROOT checked byte count"));
        }
        let len = (raw & !0x4000_0000) as usize;
        Ok(Cursor::new(self.take(len)?))
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(u64::from_be_bytes(
            self.take(8)?.try_into().unwrap(),
        )))
    }

    fn string(&mut self) -> Result<String> {
        let len = match self.u8()? {
            255 => self.u32()? as usize,
            len => len as usize,
        };
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|err| RootError::parse(err.to_string()))
    }
}

fn parse_th1f(bytes: &[u8]) -> Result<Th1F> {
    let mut top = Cursor::new(bytes);
    let mut th1f = top.checked_sub()?;
    let _version = th1f.u16()?;
    let mut th1 = th1f.checked_sub()?;
    let _th1_version = th1.u16()?;
    let (name, title) = parse_tnamed_checked(&mut th1)?;
    skip_checked(&mut th1)?;
    skip_checked(&mut th1)?;
    skip_checked(&mut th1)?;
    let _ncells = th1.i32()?;
    let axis = parse_taxis_checked(&mut th1)?;
    let _yaxis = parse_taxis_checked(&mut th1)?;
    let _zaxis = parse_taxis_checked(&mut th1)?;
    let _bar_offset = th1.i16()?;
    let _bar_width = th1.i16()?;
    let entries = th1.f64()?;
    let tsumw = th1.f64()?;
    let tsumw2 = th1.f64()?;
    let tsumwx = th1.f64()?;
    let tsumwx2 = th1.f64()?;
    let _maximum = th1.f64()?;
    let _minimum = th1.f64()?;
    let _norm_factor = th1.f64()?;
    let _contour = parse_tarray_d(&mut th1)?;
    let sumw2 = parse_tarray_d(&mut th1)?;
    let _option = th1.string()?;
    skip_checked(&mut th1)?;
    let buffer_size = th1.i32()?;
    let _speedbump = th1.u8()?;
    for _ in 0..buffer_size.max(0) {
        let _ = th1.f64()?;
    }
    let _bin_stat_err_opt = th1.i32()?;
    let _stat_overflows = th1.i32()?;

    let contents = parse_tarray_f(&mut th1f)?;
    Ok(Th1F {
        name,
        title,
        axis,
        contents,
        entries,
        tsumw,
        tsumw2,
        tsumwx,
        tsumwx2,
        sumw2,
    })
}

fn skip_checked(cur: &mut Cursor<'_>) -> Result<()> {
    let _ = cur.checked_sub()?;
    Ok(())
}

fn parse_tnamed_checked(cur: &mut Cursor<'_>) -> Result<(String, String)> {
    let mut named = cur.checked_sub()?;
    let _version = named.u16()?;
    skip_tobject(&mut named)?;
    let name = named.string()?;
    let title = named.string()?;
    Ok((name, title))
}

fn skip_tobject(cur: &mut Cursor<'_>) -> Result<()> {
    let _version = cur.u16()?;
    let _id = cur.u32()?;
    let bits = cur.u32()?;
    if bits & (1 << 4) != 0 {
        let _process_id = cur.u16()?;
    }
    Ok(())
}

fn parse_taxis_checked(cur: &mut Cursor<'_>) -> Result<Axis1D> {
    let mut axis = cur.checked_sub()?;
    let _version = axis.u16()?;
    let _named = parse_tnamed_checked(&mut axis)?;
    skip_checked(&mut axis)?;
    let nbins = axis.i32()?;
    let xmin = axis.f64()?;
    let xmax = axis.f64()?;
    let edges = parse_tarray_d(&mut axis)?;
    let _first = axis.i32()?;
    let _last = axis.i32()?;
    let _bits2 = axis.u16()?;
    let _time_display = axis.u8()?;
    let _time_format = axis.string()?;
    let _labels = axis.u32()?;
    let _mod_labs = axis.u32()?;
    Ok(Axis1D {
        nbins,
        xmin,
        xmax,
        edges,
    })
}

fn parse_tarray_d(cur: &mut Cursor<'_>) -> Result<Vec<f64>> {
    let len = cur.i32()?;
    if len < 0 {
        return Err(RootError::parse("negative TArrayD length"));
    }
    (0..len).map(|_| cur.f64()).collect()
}

fn parse_tarray_f(cur: &mut Cursor<'_>) -> Result<Vec<f32>> {
    let len = cur.i32()?;
    if len < 0 {
        return Err(RootError::parse("negative TArrayF length"));
    }
    (0..len).map(|_| cur.f32()).collect()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use crate::core::RootFile;
    use std::path::Path;

    #[tokio::test]
    async fn open_simple() {
        let path = Path::new("./src/test_data/simple.root");
        let f = RootFile::new(path).await.expect("Failed to open file");
        assert_eq!(f.items().len(), 1);
        assert_eq!(f.items()[0].tkey_hdr.obj_name, "tree");
        // Only streamers; not rules
        assert_eq!(f.streamer_infos().await.unwrap().len(), 18);
    }

    #[tokio::test]
    #[cfg(all(feature = "remote", not(target_arch = "wasm32")))]
    async fn open_esd() {
        use alice_open_data;
        let path = alice_open_data::test_file().unwrap();

        let f = RootFile::new(path.as_path())
            .await
            .expect("Failed to open file");

        assert_eq!(f.items().len(), 2);
        assert_eq!(f.items()[0].tkey_hdr.obj_name, "esdTree");
        assert_eq!(f.items()[1].tkey_hdr.obj_name, "HLTesdTree");
        assert_eq!(f.streamer_infos().await.unwrap().len(), 87);
    }
}
