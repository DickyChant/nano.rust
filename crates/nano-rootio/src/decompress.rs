use std::io::Read;

use flate2::bufread::ZlibDecoder;
use lz4_compress::decompress as lz4_decompress;
use lzma_rs::xz_decompress;

use crate::error::{Error, Result};

fn le_u24(bytes: &[u8]) -> usize {
    bytes[0] as usize | ((bytes[1] as usize) << 8) | ((bytes[2] as usize) << 16)
}

fn decode_payload(magic: &[u8], payload: &[u8]) -> Result<Vec<u8>> {
    match magic {
        b"ZL" => {
            let mut out = Vec::new();
            let mut decoder = ZlibDecoder::new(payload);
            decoder.read_to_end(&mut out)?;
            Ok(out)
        }
        b"XZ" => {
            let mut out = Vec::new();
            let mut reader = std::io::BufReader::new(payload);
            xz_decompress(&mut reader, &mut out)
                .map_err(|err| Error::Decompression(format!("xz: {err:?}")))?;
            Ok(out)
        }
        b"L4" => {
            if payload.len() < 8 {
                return Err(Error::Decompression("lz4 block is missing checksum".into()));
            }
            lz4_decompress(&payload[8..]).map_err(|err| Error::Decompression(err.to_string()))
        }
        b"ZS" => {
            let mut out = Vec::new();
            let mut decoder = ruzstd::StreamingDecoder::new(payload)
                .map_err(|err| Error::Decompression(format!("zstd init: {err}")))?;
            decoder.read_to_end(&mut out)?;
            Ok(out)
        }
        other => Err(Error::UnsupportedCompression(
            String::from_utf8_lossy(other).into_owned(),
        )),
    }
}

/// Decode one or more ROOT compressed blocks.
///
/// ROOT wraps each compression frame as:
/// `[algo:2][method:1][compressed-size:3 LE][uncompressed-size:3 LE][payload]`.
/// The small decoder is copied in spirit from the vendored reader but kept here
/// as owned code and supports concatenated ROOT blocks.
pub(crate) fn decompress_root_blocks(mut input: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    while !input.is_empty() {
        if input.len() < 9 {
            return Err(Error::Decompression(format!(
                "truncated ROOT compression block header: {} bytes",
                input.len()
            )));
        }
        let magic = &input[..2];
        let compressed_len = le_u24(&input[3..6]);
        let uncompressed_len = le_u24(&input[6..9]);
        input = &input[9..];
        if input.len() < compressed_len {
            return Err(Error::Decompression(format!(
                "compressed block declares {compressed_len} bytes, only {} remain",
                input.len()
            )));
        }
        let payload = &input[..compressed_len];
        input = &input[compressed_len..];
        let decoded = decode_payload(magic, payload)?;
        if uncompressed_len != 0 && decoded.len() != uncompressed_len {
            return Err(Error::Decompression(format!(
                "decoded {} bytes, ROOT header declared {uncompressed_len}",
                decoded.len()
            )));
        }
        out.extend_from_slice(&decoded);
    }
    Ok(out)
}
