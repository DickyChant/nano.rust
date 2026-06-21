use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use nano_core::Event;
use nano_spec::codegen::{generate_producer_source, CodegenError};
use nano_spec::ResolvedPlan;

type AnalyzeFn = unsafe extern "C" fn(
    n_muon: u32,
    muon_pt_ptr: *const f32,
    muon_pt_len: usize,
    muon_eta_ptr: *const f32,
    muon_eta_len: usize,
    out: *mut JitMuonRow,
) -> i32;

const ANALYZE_SYMBOL: &[u8] = b"nano_jit_muon_analyze\0";

/// The stable C ABI output row for the first muon JIT slice.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct JitMuonRow {
    pub n_good_muon: u32,
    pub lead_muon_pt: f32,
}

/// Runtime Cargo profile used for the generated cdylib.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitBuildProfile {
    Debug,
    Release,
}

impl JitBuildProfile {
    fn target_dir_name(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }
}

/// A compiled and loaded muon JIT kernel.
///
/// The temporary build directory is held for the lifetime of this runner because
/// the dynamic loader may need the library file to remain present. It is removed
/// when the runner is dropped.
pub struct JitMuonRunner {
    library: DynamicLibrary,
    analyze: AnalyzeFn,
    build_dir: BuildDir,
}

impl JitMuonRunner {
    /// Generate Rust from a validated muon plan, compile it as a temporary
    /// `cdylib`, load it, and bind the stable C ABI entry point.
    pub fn compile(plan: &ResolvedPlan) -> Result<Self, JitError> {
        Self::compile_with_profile(plan, JitBuildProfile::Release)
    }

    /// Like [`Self::compile`], but lets tests or callers choose debug builds to
    /// reduce compile latency.
    pub fn compile_with_profile(
        plan: &ResolvedPlan,
        profile: JitBuildProfile,
    ) -> Result<Self, JitError> {
        let generated = generate_producer_source(plan)?;
        let build_dir = BuildDir::new()?;
        write_kernel_crate(build_dir.path(), &generated)?;
        build_kernel_crate(build_dir.path(), profile)?;

        let library_path = dylib_path(build_dir.path(), profile);
        let library =
            DynamicLibrary::open(&library_path).map_err(|source| JitError::LoadLibrary {
                path: library_path,
                source,
            })?;
        let analyze = unsafe {
            // SAFETY: The generated wrapper exports this exact symbol with the
            // `AnalyzeFn` C ABI signature. We copy the function pointer while
            // retaining the `Library` in the runner to keep the code loaded.
            library.symbol::<AnalyzeFn>(ANALYZE_SYMBOL)?
        };

        Ok(Self {
            library,
            analyze,
            build_dir,
        })
    }

    /// Run the loaded kernel on a host-side `Event`.
    ///
    /// The host extracts the needed columns and crosses the dylib boundary with
    /// only plain C-compatible values. `Event` and other Rust-owned types remain
    /// on their side of the boundary.
    pub fn analyze(&self, event: &Event) -> Result<Option<JitMuonRow>, JitError> {
        let _keep_loaded = &self.library;
        let n_muon = event.scalar::<u32>("nMuon")?;
        let muon_pt = event.vector_ref::<f32>("Muon_pt")?;
        let muon_eta = event.vector_ref::<f32>("Muon_eta")?;
        self.analyze_muon_inputs(n_muon, muon_pt, muon_eta)
    }

    /// Run the loaded kernel from already-extracted plain muon inputs.
    pub fn analyze_muon_inputs(
        &self,
        n_muon: u32,
        muon_pt: &[f32],
        muon_eta: &[f32],
    ) -> Result<Option<JitMuonRow>, JitError> {
        let mut row = JitMuonRow::default();
        let status = unsafe {
            // SAFETY: The slices are valid for the duration of the call, their
            // pointers and lengths are passed together, and `row` points to
            // writable output storage.
            (self.analyze)(
                n_muon,
                muon_pt.as_ptr(),
                muon_pt.len(),
                muon_eta.as_ptr(),
                muon_eta.len(),
                &mut row,
            )
        };

        match status {
            1 => Ok(Some(row)),
            0 => Ok(None),
            other => Err(JitError::KernelStatus(other)),
        }
    }

    pub fn build_dir(&self) -> &Path {
        self.build_dir.path()
    }
}

#[derive(Debug)]
pub enum JitError {
    Codegen(CodegenError),
    Core(nano_core::NanoError),
    Io(io::Error),
    BuildFailed {
        status: Option<i32>,
        stdout: String,
        stderr: String,
    },
    LoadLibrary {
        path: PathBuf,
        source: DynamicLibraryError,
    },
    Symbol(DynamicLibraryError),
    KernelStatus(i32),
    RepoRootNotFound(PathBuf),
}

impl fmt::Display for JitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codegen(error) => write!(f, "code generation failed: {error}"),
            Self::Core(error) => write!(f, "event input extraction failed: {error}"),
            Self::Io(error) => write!(f, "JIT filesystem operation failed: {error}"),
            Self::BuildFailed {
                status,
                stdout,
                stderr,
            } => write!(
                f,
                "runtime cargo build failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
                status, stdout, stderr
            ),
            Self::LoadLibrary { path, source } => {
                write!(f, "failed to load {}: {source}", path.display())
            }
            Self::Symbol(error) => write!(f, "failed to load JIT symbol: {error}"),
            Self::KernelStatus(status) => write!(f, "JIT kernel returned status {status}"),
            Self::RepoRootNotFound(path) => {
                write!(f, "could not find workspace root from {}", path.display())
            }
        }
    }
}

impl Error for JitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Codegen(error) => Some(error),
            Self::Core(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::LoadLibrary { source, .. } => Some(source),
            Self::Symbol(error) => Some(error),
            Self::BuildFailed { .. } | Self::KernelStatus(_) | Self::RepoRootNotFound(_) => None,
        }
    }
}

impl From<CodegenError> for JitError {
    fn from(error: CodegenError) -> Self {
        Self::Codegen(error)
    }
}

impl From<nano_core::NanoError> for JitError {
    fn from(error: nano_core::NanoError) -> Self {
        Self::Core(error)
    }
}

impl From<io::Error> for JitError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<DynamicLibraryError> for JitError {
    fn from(error: DynamicLibraryError) -> Self {
        Self::Symbol(error)
    }
}

struct BuildDir {
    path: PathBuf,
}

impl BuildDir {
    fn new() -> io::Result<Self> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("nano-jit-{}-{nonce}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for BuildDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Debug)]
pub struct DynamicLibraryError(String);

impl fmt::Display for DynamicLibraryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for DynamicLibraryError {}

#[cfg(unix)]
struct DynamicLibrary {
    handle: *mut std::ffi::c_void,
}

#[cfg(unix)]
impl DynamicLibrary {
    fn open(path: &Path) -> Result<Self, DynamicLibraryError> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        const RTLD_NOW: std::ffi::c_int = 2;

        let path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| DynamicLibraryError("library path contains an interior NUL".into()))?;
        let handle = unsafe {
            // SAFETY: `path` is a valid NUL-terminated C string. `dlopen`
            // returns either a library handle or null with a diagnostic from
            // `dlerror`.
            dlopen(path.as_ptr(), RTLD_NOW)
        };
        if handle.is_null() {
            return Err(last_dynamic_library_error());
        }
        Ok(Self { handle })
    }

    unsafe fn symbol<T: Copy>(&self, symbol: &[u8]) -> Result<T, DynamicLibraryError> {
        if !symbol.ends_with(&[0]) {
            return Err(DynamicLibraryError(
                "symbol name must be NUL-terminated".into(),
            ));
        }
        unsafe {
            // SAFETY: Clear stale loader errors before `dlsym`, per the
            // platform API.
            dlerror();
        }
        let raw = unsafe {
            // SAFETY: `self.handle` is a live handle and `symbol` is
            // NUL-terminated. A null return is checked below.
            dlsym(self.handle, symbol.as_ptr().cast())
        };
        if raw.is_null() {
            return Err(last_dynamic_library_error());
        }

        Ok(unsafe {
            // SAFETY: The caller chooses `T` to match the exported symbol. This
            // module calls it only for `AnalyzeFn`, the wrapper we generate.
            std::mem::transmute_copy::<*mut std::ffi::c_void, T>(&raw)
        })
    }
}

#[cfg(unix)]
impl Drop for DynamicLibrary {
    fn drop(&mut self) {
        unsafe {
            // SAFETY: `handle` was returned by `dlopen` and is owned by this
            // wrapper.
            dlclose(self.handle);
        }
    }
}

#[cfg(unix)]
fn last_dynamic_library_error() -> DynamicLibraryError {
    let error = unsafe {
        // SAFETY: `dlerror` returns a thread-local diagnostic pointer or null.
        dlerror()
    };
    if error.is_null() {
        return DynamicLibraryError("dynamic loader returned no error detail".into());
    }

    let message = unsafe {
        // SAFETY: Non-null `dlerror` pointers are NUL-terminated strings owned
        // by the dynamic loader for immediate diagnostic use.
        std::ffi::CStr::from_ptr(error)
    }
    .to_string_lossy()
    .into_owned();
    DynamicLibraryError(message)
}

#[cfg(unix)]
#[link(name = "dl")]
extern "C" {
    fn dlopen(filename: *const std::ffi::c_char, flags: std::ffi::c_int) -> *mut std::ffi::c_void;
    fn dlsym(
        handle: *mut std::ffi::c_void,
        symbol: *const std::ffi::c_char,
    ) -> *mut std::ffi::c_void;
    fn dlclose(handle: *mut std::ffi::c_void) -> std::ffi::c_int;
    fn dlerror() -> *const std::ffi::c_char;
}

#[cfg(not(unix))]
struct DynamicLibrary;

#[cfg(not(unix))]
impl DynamicLibrary {
    fn open(_path: &Path) -> Result<Self, DynamicLibraryError> {
        Err(DynamicLibraryError(
            "nano-jit dynamic loading is implemented for Unix targets in this slice".into(),
        ))
    }

    unsafe fn symbol<T: Copy>(&self, _symbol: &[u8]) -> Result<T, DynamicLibraryError> {
        Err(DynamicLibraryError(
            "nano-jit dynamic loading is implemented for Unix targets in this slice".into(),
        ))
    }
}

fn write_kernel_crate(build_dir: &Path, generated: &str) -> Result<(), JitError> {
    let src_dir = build_dir.join("src");
    fs::create_dir(&src_dir)?;
    fs::write(build_dir.join("Cargo.toml"), kernel_cargo_toml()?)?;
    let root = workspace_root()?;
    let lockfile = root.join("Cargo.lock");
    if lockfile.exists() {
        fs::copy(lockfile, build_dir.join("Cargo.lock"))?;
    }
    fs::write(src_dir.join("generated.rs"), generated)?;
    fs::write(src_dir.join("lib.rs"), kernel_lib_rs())?;
    Ok(())
}

fn build_kernel_crate(build_dir: &Path, profile: JitBuildProfile) -> Result<(), JitError> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsStr::new("cargo").to_os_string());
    let mut command = Command::new(cargo);
    command
        .arg("build")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(build_dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(build_dir.join("target"));
    if std::env::var("NANO_JIT_CARGO_OFFLINE").as_deref() == Ok("1") {
        command.arg("--offline");
    }
    if profile == JitBuildProfile::Release {
        command.arg("--release");
    }

    let output = command.output()?;
    if output.status.success() {
        return Ok(());
    }

    Err(JitError::BuildFailed {
        status: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn kernel_cargo_toml() -> Result<String, JitError> {
    let root = workspace_root()?;
    Ok(format!(
        r#"[package]
name = "nano-jit-muon-kernel"
version = "0.0.0"
edition = "2021"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
nano-analysis = {{ path = "{}" }}
nano-core = {{ path = "{}" }}
"#,
        toml_path(root.join("crates/nano-analysis")),
        toml_path(root.join("crates/nano-core")),
    ))
}

fn kernel_lib_rs() -> &'static str {
    r#"#![allow(
    dead_code,
    non_snake_case,
    unused_parens,
    clippy::collapsible_if,
    clippy::double_parens,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::unnecessary_cast
)]

include!("generated.rs");

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct JitMuonRow {
    pub n_good_muon: u32,
    pub lead_muon_pt: f32,
}

#[no_mangle]
pub unsafe extern "C" fn nano_jit_muon_analyze(
    n_muon: u32,
    muon_pt_ptr: *const f32,
    muon_pt_len: usize,
    muon_eta_ptr: *const f32,
    muon_eta_len: usize,
    out: *mut JitMuonRow,
) -> i32 {
    if out.is_null() {
        return -1;
    }
    if (muon_pt_ptr.is_null() && muon_pt_len != 0) || (muon_eta_ptr.is_null() && muon_eta_len != 0) {
        return -1;
    }
    if muon_pt_len != muon_eta_len || n_muon as usize != muon_pt_len {
        return -2;
    }

    let result = std::panic::catch_unwind(|| {
        let muon_pt = std::slice::from_raw_parts(muon_pt_ptr, muon_pt_len).to_vec();
        let muon_eta = std::slice::from_raw_parts(muon_eta_ptr, muon_eta_len).to_vec();
        let schema = nano_core::BranchSchema::new([
            nano_core::BranchSpec::new("nMuon", nano_core::BranchType::U32),
            nano_core::BranchSpec::new("Muon_pt", nano_core::BranchType::VecF32),
            nano_core::BranchSpec::new("Muon_eta", nano_core::BranchType::VecF32),
        ])
        .map_err(|_| -3)?;
        let event = nano_core::Event::from_columns(
            schema,
            [
                ("nMuon", nano_core::BranchColumn::U32(vec![n_muon])),
                ("Muon_pt", nano_core::BranchColumn::VecF32(vec![muon_pt])),
                ("Muon_eta", nano_core::BranchColumn::VecF32(vec![muon_eta])),
            ],
            0,
        )
        .map_err(|_| -3)?;
        GeneratedProducer::analyze(&event).map_err(|_| -3)
    });

    match result {
        Ok(Ok(Some(row))) => {
            *out = JitMuonRow {
                n_good_muon: row.n_good_muon,
                lead_muon_pt: row.lead_muon_pt,
            };
            1
        }
        Ok(Ok(None)) => 0,
        Ok(Err(status)) => status,
        Err(_) => -4,
    }
}
"#
}

fn workspace_root() -> Result<PathBuf, JitError> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or(JitError::RepoRootNotFound(manifest_dir))
}

fn toml_path(path: PathBuf) -> String {
    path.to_string_lossy().replace('\\', "\\\\")
}

fn dylib_path(build_dir: &Path, profile: JitBuildProfile) -> PathBuf {
    build_dir
        .join("target")
        .join(profile.target_dir_name())
        .join(dylib_file_name("nano_jit_muon_kernel"))
}

fn dylib_file_name(crate_name: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{crate_name}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{crate_name}.dylib")
    } else {
        format!("lib{crate_name}.so")
    }
}
