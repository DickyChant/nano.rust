use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;
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

use crate::Result;
#[cfg(feature = "http")]
use crate::RootError;

#[cfg(feature = "http")]
const USER_AGENT: &str = "nano.rust root-io/0.3";
#[cfg(feature = "http")]
const MAX_REDIRECTS: usize = 10;

/// The source from where the Root file is read. Construct it using
/// `.into()` on a `Path`, or [`Source::http`] when the `http` feature is
/// enabled. Local `Path` sources are not available for the `wasm32` target.
#[derive(Clone)]
pub struct Source {
    inner: SourceInner,
    bytes_fetched: Arc<AtomicU64>,
}

// This inner enum hides the differentiation between the local and
// remote files from the public API
#[derive(Clone)]
enum SourceInner {
    /// A local source, i.e. a file on disc.
    Local(PathBuf),
    #[cfg(feature = "http")]
    Http { agent: Agent, url: Url },
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
    /// Accept invalid TLS certificates. This is suitable only for public,
    /// read-only data where the caller accepts transport authenticity risk.
    pub insecure: bool,
}

#[cfg(feature = "http")]
impl HttpSourceOptions {
    /// Build options from process environment.
    ///
    /// `SSL_CERT_FILE` points to a PEM CA bundle. `NANO_HTTP_INSECURE=1` (also
    /// `true` or `yes`) disables TLS certificate verification.
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
        let url = Url::parse(url)
            .map_err(|err| RootError::other(format!("invalid HTTP ROOT URL `{url}`: {err}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(RootError::other(format!(
                "HTTP ROOT source requires http:// or https:// URL, got `{}`",
                url.scheme()
            )));
        }
        let agent = http_agent(options)?;
        Ok(Self {
            inner: SourceInner::Http { agent, url },
            bytes_fetched: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Bytes returned by this source's fetches. Clones share the same counter.
    pub fn bytes_fetched(&self) -> u64 {
        self.bytes_fetched.load(Ordering::Relaxed)
    }

    pub async fn fetch(&self, start: u64, len: u64) -> Result<Vec<u8>> {
        if len == 0 {
            return Ok(Vec::new());
        }

        let bytes = match &self.inner {
            SourceInner::Local(path) => {
                let mut f = File::open(path)?;
                f.seek(SeekFrom::Start(start))?;
                let mut buf = vec![0; len as usize];
                f.read_exact(&mut buf)?;
                buf
            }
            #[cfg(feature = "http")]
            SourceInner::Http { agent, url } => fetch_http(agent, url, start, len)?,
        };
        self.bytes_fetched
            .fetch_add(bytes.len() as u64, Ordering::Relaxed);
        Ok(bytes)
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
fn ca_bundle_tls_config(path: &PathBuf) -> Result<ClientConfig> {
    let file = File::open(path).map_err(|err| {
        RootError::other(format!(
            "failed to open TLS CA bundle `{}`: {err}",
            path.display()
        ))
    })?;
    let mut reader = BufReader::new(file);
    let certs = rustls_pemfile::certs(&mut reader).map_err(|err| {
        RootError::other(format!(
            "failed to read TLS CA bundle `{}`: {err}",
            path.display()
        ))
    })?;
    let mut roots = RootCertStore::empty();
    for cert in certs {
        roots.add(&Certificate(cert)).map_err(|err| {
            RootError::other(format!(
                "failed to add certificate from TLS CA bundle `{}`: {err}",
                path.display()
            ))
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
        .ok_or_else(|| RootError::other("HTTP range overflow"))?;
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
                return Err(RootError::other(format!(
                    "HTTP range request failed for `{url}` ({range}): {err}"
                )));
            }
        };

        if is_redirect(response.status()) {
            let location = response.header("Location").ok_or_else(|| {
                RootError::other(format!(
                    "HTTP redirect from `{url}` did not include a Location header"
                ))
            })?;
            url = url.join(location).map_err(|err| {
                RootError::other(format!(
                    "invalid HTTP redirect Location `{location}` from `{url}`: {err}"
                ))
            })?;
            continue;
        }

        return read_partial_response(response, &url, &range, len);
    }

    Err(RootError::other(format!(
        "too many HTTP redirects while fetching `{original_url}`"
    )))
}

#[cfg(feature = "http")]
fn read_partial_response(
    response: Response,
    url: &Url,
    range: &str,
    expected_len: u64,
) -> Result<Vec<u8>> {
    if response.status() != 206 {
        return Err(RootError::other(format!(
            "HTTP range request for `{url}` ({range}) returned status {}, expected 206 Partial Content",
            response.status()
        )));
    }

    let mut bytes = Vec::with_capacity(expected_len as usize);
    response.into_reader().read_to_end(&mut bytes)?;
    if bytes.len() as u64 != expected_len {
        return Err(RootError::other(format!(
            "HTTP range request for `{url}` ({range}) returned {} bytes, expected {expected_len}",
            bytes.len()
        )));
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

// Disallow the construction of a local source object on wasm since
// wasm does not have a (proper) file system.
#[cfg(not(target_arch = "wasm32"))]
impl From<&Path> for Source {
    fn from(path: &Path) -> Self {
        path.to_path_buf().into()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<PathBuf> for Source {
    fn from(path_buf: PathBuf) -> Self {
        Self {
            inner: SourceInner::Local(path_buf),
            bytes_fetched: Arc::new(AtomicU64::new(0)),
        }
    }
}
