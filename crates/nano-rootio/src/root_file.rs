use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[cfg(feature = "http")]
use std::io::BufReader;
#[cfg(feature = "http")]
use std::time::{Duration, SystemTime};

#[cfg(feature = "http")]
use rustls::client::{ServerCertVerified, ServerCertVerifier};
#[cfg(feature = "http")]
use rustls::{Certificate, ClientConfig, RootCertStore, ServerName};
#[cfg(feature = "http")]
use ureq::{Agent, AgentBuilder, Response};
#[cfg(feature = "http")]
use url::Url;

use crate::decompress::decompress_root_blocks;
use crate::error::{Error, Result};
use crate::parse::{Cursor, ObjectContext, TBUFFER_OBJECT_MAP_OFFSET};
use crate::tree::{parse_tree, Tree};

const FILE_HEADER_SIZE: usize = 75;
const TDIRECTORY_MAX_SIZE: usize = 42;
#[cfg(feature = "http")]
const USER_AGENT: &str = "nano.rust nano-rootio/0.0";
#[cfg(feature = "http")]
const MAX_REDIRECTS: usize = 10;

/// Byte-range source for ROOT reads.
#[derive(Clone)]
pub struct Source {
    inner: SourceInner,
    bytes_fetched: Arc<AtomicU64>,
}

#[derive(Clone)]
enum SourceInner {
    Local(Arc<PathBuf>),
    #[cfg(feature = "http")]
    Http {
        agent: Agent,
        url: Url,
    },
}

impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Source")
            .field("inner", &self.inner)
            .field("bytes_fetched", &self.bytes_fetched())
            .finish()
    }
}

impl fmt::Debug for SourceInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local(path) => f.debug_tuple("Local").field(path).finish(),
            #[cfg(feature = "http")]
            Self::Http { url, .. } => f.debug_struct("Http").field("url", url).finish(),
        }
    }
}

/// Configuration for an HTTP(S) byte-range source.
#[cfg(feature = "http")]
#[derive(Debug, Clone, Default)]
pub struct HttpSourceOptions {
    /// PEM certificate bundle used for TLS verification. When unset, ureq's
    /// default rustls root store is used. [`HttpSourceOptions::from_env`]
    /// populates this from `SSL_CERT_FILE`.
    pub ca_bundle: Option<PathBuf>,
    /// Accept invalid TLS certificates. Use only for public, read-only data.
    pub insecure: bool,
}

#[cfg(feature = "http")]
impl HttpSourceOptions {
    /// Build options from process environment.
    ///
    /// `SSL_CERT_FILE` points to a PEM CA bundle. `NANO_HTTP_INSECURE=1`
    /// (also `true` or `yes`) disables TLS certificate verification.
    pub fn from_env() -> Self {
        Self {
            ca_bundle: std::env::var_os("SSL_CERT_FILE").map(PathBuf::from),
            insecure: env_flag("NANO_HTTP_INSECURE"),
        }
    }

    pub fn with_ca_bundle(mut self, path: impl Into<PathBuf>) -> Self {
        self.ca_bundle = Some(path.into());
        self
    }

    pub fn insecure(mut self, insecure: bool) -> Self {
        self.insecure = insecure;
        self
    }
}

impl Source {
    pub fn new<T: Into<Self>>(thing: T) -> Self {
        thing.into()
    }

    /// Build an HTTP(S) byte-range source using `SSL_CERT_FILE` and
    /// `NANO_HTTP_INSECURE` from the environment.
    #[cfg(feature = "http")]
    pub fn http(url: &str) -> Result<Self> {
        Self::http_with_options(url, HttpSourceOptions::from_env())
    }

    /// Build an HTTP(S) byte-range source with explicit TLS options.
    #[cfg(feature = "http")]
    pub fn http_with_options(url: &str, options: HttpSourceOptions) -> Result<Self> {
        let url = Url::parse(url).map_err(|err| {
            Error::unsupported("HTTP ROOT source", format!("invalid URL `{url}`: {err}"))
        })?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(Error::unsupported(
                "HTTP ROOT source",
                format!("requires http:// or https:// URL, got `{}`", url.scheme()),
            ));
        }
        Ok(Self {
            inner: SourceInner::Http {
                agent: http_agent(options)?,
                url,
            },
            bytes_fetched: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Bytes returned by this source's fetches. Clones share the same counter.
    pub fn bytes_fetched(&self) -> u64 {
        self.bytes_fetched.load(Ordering::Relaxed)
    }

    pub fn fetch(&self, offset: u64, len: u64) -> Result<Vec<u8>> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let len = usize::try_from(len)
            .map_err(|_| Error::parse(0, format!("read length {len} overflows usize")))?;
        let out = match &self.inner {
            SourceInner::Local(path) => {
                let mut file = File::open(&**path)?;
                file.seek(SeekFrom::Start(offset))?;
                let mut out = vec![0; len];
                file.read_exact(&mut out)?;
                out
            }
            #[cfg(feature = "http")]
            SourceInner::Http { agent, url } => fetch_http(agent, url, offset, len as u64)?,
        };
        self.bytes_fetched
            .fetch_add(out.len() as u64, Ordering::Relaxed);
        Ok(out)
    }
}

impl From<&Path> for Source {
    fn from(path: &Path) -> Self {
        path.to_path_buf().into()
    }
}

impl From<PathBuf> for Source {
    fn from(path: PathBuf) -> Self {
        Self {
            inner: SourceInner::Local(Arc::new(path)),
            bytes_fetched: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[cfg(feature = "http")]
fn http_agent(options: HttpSourceOptions) -> Result<Agent> {
    let mut builder = AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_secs(30))
        .timeout_read(Duration::from_secs(120));

    if options.insecure || options.ca_bundle.is_some() {
        let config = if options.insecure {
            insecure_tls_config()
        } else {
            ca_bundle_tls_config(
                options
                    .ca_bundle
                    .as_ref()
                    .expect("checked ca_bundle presence"),
            )?
        };
        builder = builder.tls_config(Arc::new(config));
    }

    Ok(builder.build())
}

#[cfg(feature = "http")]
fn ca_bundle_tls_config(path: &Path) -> Result<ClientConfig> {
    let file = File::open(path).map_err(|err| {
        Error::unsupported(
            "HTTP ROOT source",
            format!("failed to open TLS CA bundle `{}`: {err}", path.display()),
        )
    })?;
    let mut reader = BufReader::new(file);
    let certs = rustls_pemfile::certs(&mut reader).map_err(|err| {
        Error::unsupported(
            "HTTP ROOT source",
            format!("failed to read TLS CA bundle `{}`: {err}", path.display()),
        )
    })?;
    let mut roots = RootCertStore::empty();
    for cert in certs {
        roots.add(&Certificate(cert)).map_err(|err| {
            Error::unsupported(
                "HTTP ROOT source",
                format!(
                    "failed to add certificate from TLS CA bundle `{}`: {err}",
                    path.display()
                ),
            )
        })?;
    }

    Ok(ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(roots)
        .with_no_client_auth())
}

#[cfg(feature = "http")]
fn insecure_tls_config() -> ClientConfig {
    let mut config = ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(RootCertStore::empty())
        .with_no_client_auth();
    config
        .dangerous()
        .set_certificate_verifier(Arc::new(NoCertificateVerification));
    config
}

#[cfg(feature = "http")]
#[derive(Debug)]
struct NoCertificateVerification;

#[cfg(feature = "http")]
impl ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &Certificate,
        _intermediates: &[Certificate],
        _server_name: &ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: SystemTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
}

#[cfg(feature = "http")]
fn fetch_http(agent: &Agent, original_url: &Url, start: u64, len: u64) -> Result<Vec<u8>> {
    let end = start
        .checked_add(len)
        .and_then(|value| value.checked_sub(1))
        .ok_or_else(|| Error::unsupported("HTTP ROOT source", "range overflow"))?;
    let range = format!("bytes={start}-{end}");
    let mut url = original_url.clone();

    for _ in 0..=MAX_REDIRECTS {
        let response = match agent
            .get(url.as_str())
            .set("User-Agent", USER_AGENT)
            .set("Range", &range)
            .call()
        {
            Ok(response) => response,
            Err(ureq::Error::Status(_, response)) => response,
            Err(err) => {
                return Err(Error::unsupported(
                    "HTTP ROOT source",
                    format!("HTTP range request failed for `{url}` ({range}): {err}"),
                ));
            }
        };

        if is_redirect(response.status()) {
            let location = response.header("Location").ok_or_else(|| {
                Error::unsupported(
                    "HTTP ROOT source",
                    format!("HTTP redirect from `{url}` did not include a Location header"),
                )
            })?;
            url = url.join(location).map_err(|err| {
                Error::unsupported(
                    "HTTP ROOT source",
                    format!("invalid HTTP redirect Location `{location}` from `{url}`: {err}"),
                )
            })?;
            continue;
        }

        return read_partial_response(response, &url, &range, len);
    }

    Err(Error::unsupported(
        "HTTP ROOT source",
        format!("too many HTTP redirects while fetching `{original_url}`"),
    ))
}

#[cfg(feature = "http")]
fn read_partial_response(
    response: Response,
    url: &Url,
    range: &str,
    expected_len: u64,
) -> Result<Vec<u8>> {
    if response.status() != 206 {
        return Err(Error::unsupported(
            "HTTP ROOT source",
            format!(
                "HTTP range request for `{url}` ({range}) returned status {}, expected 206 Partial Content",
                response.status()
            ),
        ));
    }

    let mut bytes = Vec::with_capacity(expected_len as usize);
    response.into_reader().read_to_end(&mut bytes)?;
    if bytes.len() as u64 != expected_len {
        return Err(Error::unsupported(
            "HTTP ROOT source",
            format!(
                "HTTP range request for `{url}` ({range}) returned {} bytes, expected {expected_len}",
                bytes.len()
            ),
        ));
    }
    Ok(bytes)
}

#[cfg(feature = "http")]
fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

#[cfg(feature = "http")]
fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
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
    source: Source,
    header: FileHeader,
    items: Vec<FileItem>,
    streamer_infos: Vec<StreamerInfo>,
}

impl RootFile {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_source(Source::new(path.as_ref().to_path_buf()))
    }

    /// Open a ROOT file from an HTTP(S) URL using environment-driven TLS options.
    #[cfg(feature = "http")]
    pub fn open_url(url: &str) -> Result<Self> {
        Self::from_source(Source::http(url)?)
    }

    /// Open a ROOT file from an HTTP(S) URL using explicit TLS options.
    #[cfg(feature = "http")]
    pub fn open_url_with_options(url: &str, options: HttpSourceOptions) -> Result<Self> {
        Self::from_source(Source::http_with_options(url, options)?)
    }

    /// Open a ROOT file from an existing byte-range source.
    pub fn from_source(source: Source) -> Result<Self> {
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
            source,
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

    pub fn bytes_fetched(&self) -> u64 {
        self.source.bytes_fetched()
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
