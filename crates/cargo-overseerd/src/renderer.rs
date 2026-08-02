use std::ffi::OsStr;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use overseerd_tooling_schema::ToolingDocument;
use overseerd_tooling_schema::renderer::{
    RendererManifest, RendererPresentation, RendererRequest, RendererResponse,
    RendererValidationError, RendererView, TOOLING_RENDERER_ARGUMENT, TOOLING_RENDERER_REQUEST_ENV,
    TOOLING_RENDERER_RESPONSE_ENV,
};
use thiserror::Error;

use crate::CancellationToken;
use crate::process::{ProcessExecutionError, execute_silent_with_timeout};

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_EXCHANGE_BYTES: u64 = 16 * 1024 * 1024;
const RENDERER_TIMEOUT: Duration = Duration::from_secs(5);
const RUN_DIRECTORY_ATTEMPTS: u16 = 128;
const RESERVED_RENDERER_ENV_PREFIX: &str = "OVERSEERD_TOOLING_RENDERER_";
static NEXT_RUN_DIRECTORY: AtomicU64 = AtomicU64::new(0);

/// One validated explicitly authorized local display renderer.
#[derive(Clone, Debug)]
pub struct DisplayRenderer {
    /// Stable contract manifest.
    pub manifest: RendererManifest,
    /// Absolute executable path resolved against the manifest location.
    pub executable: PathBuf,
    /// Manifest path used for actionable diagnostics.
    pub manifest_path: PathBuf,
}

/// Failure while loading one explicit renderer manifest.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RendererManifestError {
    /// Manifest path is not absolute after deterministic resolution.
    #[error("renderer manifest path is not absolute")]
    RelativeManifest,
    /// Manifest metadata could not be read.
    #[error("failed to inspect renderer manifest")]
    Metadata(#[source] std::io::Error),
    /// Manifest is not a regular file.
    #[error("renderer manifest is not a regular file")]
    NotFile,
    /// Manifest exceeds the bounded contract size.
    #[error("renderer manifest exceeds {MAX_MANIFEST_BYTES} bytes")]
    TooLarge,
    /// Manifest bytes could not be read.
    #[error("failed to read renderer manifest")]
    Read(#[source] std::io::Error),
    /// Manifest is not UTF-8 JSON.
    #[error("renderer manifest is not UTF-8")]
    Encoding(#[source] std::string::FromUtf8Error),
    /// Manifest JSON could not be decoded.
    #[error("renderer manifest is invalid JSON")]
    Decode(#[source] serde_json::Error),
    /// Manifest violates the renderer contract.
    #[error("renderer manifest violates the display-renderer contract")]
    Validation(#[source] RendererValidationError),
    /// Relative executable cannot be resolved without a manifest parent.
    #[error("renderer manifest has no parent directory")]
    MissingParent,
    /// Renderer executable path is absent or not a regular file.
    #[error("renderer executable is not a regular file")]
    InvalidExecutable,
    /// Two manifests declare the same renderer identity.
    #[error("duplicate renderer identity '{id}'")]
    DuplicateId {
        /// Duplicated renderer identity.
        id: String,
    },
    /// Two manifests claim the same exact owner.
    #[error("duplicate renderer owner '{owner}'")]
    DuplicateOwner {
        /// Duplicated owner identity.
        owner: String,
    },
}

/// Stable category for one discarded renderer invocation.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RendererDiagnosticCode {
    /// Renderer is incompatible with the document or owner facets.
    Incompatible,
    /// Renderer process could not be launched or monitored.
    Process,
    /// Renderer exceeded its finite invocation deadline.
    Timeout,
    /// Renderer invocation was cancelled by the caller.
    Cancelled,
    /// Renderer returned unsuccessful process status.
    Failed,
    /// Renderer did not publish a response.
    MissingResponse,
    /// Renderer response exceeded the contract size.
    ResponseTooLarge,
    /// Renderer response could not be consumed.
    InvalidResponse,
}

impl RendererDiagnosticCode {
    /// Stable namespaced diagnostic identity.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Incompatible => "overseerd/renderer-incompatible",
            Self::Process => "overseerd/renderer-process",
            Self::Timeout => "overseerd/renderer-timeout",
            Self::Cancelled => "overseerd/renderer-cancelled",
            Self::Failed => "overseerd/renderer-failed",
            Self::MissingResponse => "overseerd/renderer-missing-response",
            Self::ResponseTooLarge => "overseerd/renderer-response-too-large",
            Self::InvalidResponse => "overseerd/renderer-invalid-response",
        }
    }
}

/// Deterministic presentation-side diagnostic that never mutates the tooling document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererDiagnostic {
    /// Stable renderer identity.
    pub renderer: String,
    /// Stable failure category.
    pub code: RendererDiagnosticCode,
    /// Actionable bounded message without renderer-provided stderr.
    pub message: String,
}

/// Combined validated overlays and non-authoritative renderer diagnostics.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RendererRun {
    /// Successfully validated presentation overlays.
    pub presentation: RendererPresentation,
    /// Failures discarded in deterministic renderer order.
    pub diagnostics: Vec<RendererDiagnostic>,
}

/// Loads and validates repeatable explicit renderer manifests.
pub fn load_renderers(
    paths: impl IntoIterator<Item = PathBuf>,
) -> Result<Vec<DisplayRenderer>, RendererManifestError> {
    let mut renderers = paths
        .into_iter()
        .map(load_renderer)
        .collect::<Result<Vec<_>, _>>()?;

    renderers.sort_by(|left, right| {
        (&left.manifest.owner, &left.manifest.id, &left.manifest_path).cmp(&(
            &right.manifest.owner,
            &right.manifest.id,
            &right.manifest_path,
        ))
    });

    let mut ids = std::collections::BTreeSet::new();
    let mut owners = std::collections::BTreeSet::new();

    for renderer in &renderers {
        if !ids.insert(renderer.manifest.id.as_str()) {
            return Err(RendererManifestError::DuplicateId {
                id: renderer.manifest.id.clone(),
            });
        }

        if !owners.insert(renderer.manifest.owner.as_str()) {
            return Err(RendererManifestError::DuplicateOwner {
                owner: renderer.manifest.owner.clone(),
            });
        }
    }

    Ok(renderers)
}

/// Invokes compatible renderers sequentially and falls back independently on every failure.
pub fn run_renderers(
    renderers: &[DisplayRenderer],
    document: &ToolingDocument,
    view: RendererView,
    resources: impl IntoIterator<Item = String> + Clone,
    workspace_target_directory: &Path,
    cancellation: &CancellationToken,
) -> RendererRun {
    let mut run = RendererRun::default();

    for renderer in renderers {
        if cancellation.is_cancelled() {
            run.diagnostics
                .push(InvocationError::Cancelled.diagnostic(renderer));

            break;
        }

        let request =
            match RendererRequest::new(&renderer.manifest, document, view, resources.clone()) {
                Ok(request) => request,
                Err(error) => {
                    run.diagnostics.push(diagnostic(
                        renderer,
                        RendererDiagnosticCode::Incompatible,
                        error,
                    ));

                    continue;
                }
            };

        match invoke(renderer, &request, workspace_target_directory, cancellation) {
            Ok(presentation) => {
                if let Err(error) = run.presentation.merge(presentation) {
                    run.diagnostics.push(diagnostic(
                        renderer,
                        RendererDiagnosticCode::InvalidResponse,
                        error,
                    ));
                }
            }
            Err(InvocationError::Cancelled) => {
                run.diagnostics
                    .push(InvocationError::Cancelled.diagnostic(renderer));

                break;
            }
            Err(error) => run.diagnostics.push(error.diagnostic(renderer)),
        }
    }

    run
}

fn load_renderer(path: PathBuf) -> Result<DisplayRenderer, RendererManifestError> {
    let path = absolute_path(path)?;
    let bytes = read_bounded(&path, MAX_MANIFEST_BYTES).map_err(|error| match error {
        ReadError::Metadata(source) => RendererManifestError::Metadata(source),
        ReadError::NotFile => RendererManifestError::NotFile,
        ReadError::TooLarge => RendererManifestError::TooLarge,
        ReadError::Read(source) => RendererManifestError::Read(source),
    })?;
    let json = String::from_utf8(bytes).map_err(RendererManifestError::Encoding)?;
    let manifest: RendererManifest =
        serde_json::from_str(&json).map_err(RendererManifestError::Decode)?;

    manifest
        .validate()
        .map_err(RendererManifestError::Validation)?;

    let configured = PathBuf::from(&manifest.executable);
    let executable = if configured.is_absolute() {
        configured
    } else {
        path.parent()
            .ok_or(RendererManifestError::MissingParent)?
            .join(configured)
    };

    if !executable
        .metadata()
        .is_ok_and(|metadata| metadata.is_file())
    {
        return Err(RendererManifestError::InvalidExecutable);
    }

    Ok(DisplayRenderer {
        manifest,
        executable,
        manifest_path: path,
    })
}

fn invoke(
    renderer: &DisplayRenderer,
    request: &RendererRequest,
    workspace_target_directory: &Path,
    cancellation: &CancellationToken,
) -> Result<RendererPresentation, InvocationError> {
    let directory = RendererDirectory::create(workspace_target_directory)?;
    let request_path = directory.path.join("request.json");
    let response_path = directory.path.join("response.json");
    let request_json = request.to_json().map_err(InvocationError::Request)?;

    std::fs::write(&request_path, request_json).map_err(InvocationError::WriteRequest)?;

    let mut command = Command::new(&renderer.executable);

    command.arg(TOOLING_RENDERER_ARGUMENT);
    configure_renderer_environment(&mut command);
    command
        .env(TOOLING_RENDERER_REQUEST_ENV, &request_path)
        .env(TOOLING_RENDERER_RESPONSE_ENV, &response_path)
        .current_dir(&directory.path);

    let output = execute_silent_with_timeout(&mut command, cancellation, RENDERER_TIMEOUT)
        .map_err(InvocationError::Process)?;

    if output.cancelled {
        return Err(InvocationError::Cancelled);
    }

    if output.timed_out {
        return Err(InvocationError::Timeout);
    }

    if !output.status.success {
        return Err(InvocationError::Failed);
    }

    let bytes = read_bounded(&response_path, MAX_EXCHANGE_BYTES).map_err(|error| match error {
        ReadError::Metadata(source) | ReadError::Read(source) => {
            InvocationError::ReadResponse(source)
        }
        ReadError::NotFile => InvocationError::InvalidResponseFile,
        ReadError::TooLarge => InvocationError::ResponseTooLarge,
    })?;
    let json = String::from_utf8(bytes).map_err(InvocationError::Encoding)?;
    let response =
        RendererResponse::from_json(json.trim_end(), request).map_err(InvocationError::Response)?;
    let cleanup_error = directory.close().err();

    if let Some(error) = cleanup_error {
        return Err(InvocationError::Cleanup(error));
    }

    Ok(response.presentation)
}

#[derive(Debug, Error)]
enum InvocationError {
    #[error("failed to create renderer exchange directory")]
    CreateDirectory(#[source] std::io::Error),
    #[error("exhausted renderer exchange directory names")]
    DirectoryExhausted,
    #[error("failed to encode renderer request")]
    Request(#[source] overseerd_tooling_schema::renderer::RendererEmitError),
    #[error("failed to write renderer request")]
    WriteRequest(#[source] std::io::Error),
    #[error("failed to execute renderer process")]
    Process(#[source] ProcessExecutionError),
    #[error("renderer exceeded its five-second deadline")]
    Timeout,
    #[error("renderer invocation was cancelled")]
    Cancelled,
    #[error("renderer exited unsuccessfully")]
    Failed,
    #[error("renderer response is absent or unreadable")]
    ReadResponse(#[source] std::io::Error),
    #[error("renderer response is not a regular file")]
    InvalidResponseFile,
    #[error("renderer response exceeds {MAX_EXCHANGE_BYTES} bytes")]
    ResponseTooLarge,
    #[error("renderer response is not UTF-8")]
    Encoding(#[source] std::string::FromUtf8Error),
    #[error("renderer response violates the presentation contract")]
    Response(#[source] overseerd_tooling_schema::renderer::RendererDecodeError),
    #[error("failed to remove renderer exchange directory")]
    Cleanup(#[source] std::io::Error),
}

impl InvocationError {
    fn diagnostic(self, renderer: &DisplayRenderer) -> RendererDiagnostic {
        let code = match self {
            Self::Timeout => RendererDiagnosticCode::Timeout,
            Self::Cancelled => RendererDiagnosticCode::Cancelled,
            Self::Failed => RendererDiagnosticCode::Failed,
            Self::ReadResponse(ref source) if source.kind() == std::io::ErrorKind::NotFound => {
                RendererDiagnosticCode::MissingResponse
            }
            Self::ResponseTooLarge => RendererDiagnosticCode::ResponseTooLarge,
            Self::Response(_) | Self::Encoding(_) | Self::InvalidResponseFile => {
                RendererDiagnosticCode::InvalidResponse
            }
            _ => RendererDiagnosticCode::Process,
        };

        RendererDiagnostic {
            renderer: renderer.manifest.id.clone(),
            code,
            message: self.to_string(),
        }
    }
}

fn diagnostic(
    renderer: &DisplayRenderer,
    code: RendererDiagnosticCode,
    error: impl std::fmt::Display,
) -> RendererDiagnostic {
    RendererDiagnostic {
        renderer: renderer.manifest.id.clone(),
        code,
        message: error.to_string(),
    }
}

fn absolute_path(path: PathBuf) -> Result<PathBuf, RendererManifestError> {
    if path.is_absolute() {
        return Ok(path);
    }

    let current = std::env::current_dir().map_err(RendererManifestError::Metadata)?;
    let absolute = current.join(path);

    if !absolute.is_absolute() {
        return Err(RendererManifestError::RelativeManifest);
    }

    Ok(absolute)
}

enum ReadError {
    Metadata(std::io::Error),
    NotFile,
    TooLarge,
    Read(std::io::Error),
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, ReadError> {
    let mut file = open_exchange_file(path).map_err(ReadError::Read)?;
    let metadata = file.metadata().map_err(ReadError::Metadata)?;

    if !metadata.is_file() {
        return Err(ReadError::NotFile);
    }

    if metadata.len() > limit {
        return Err(ReadError::TooLarge);
    }

    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    let mut bounded = (&mut file).take(limit + 1);

    bounded.read_to_end(&mut bytes).map_err(ReadError::Read)?;

    if bytes.len() as u64 > limit {
        return Err(ReadError::TooLarge);
    }

    Ok(bytes)
}

fn configure_renderer_environment(command: &mut Command) {
    const ALLOWED_ENVIRONMENT: &[&str] = &["PATH", "SystemRoot", "WINDIR"];

    let allowed = ALLOWED_ENVIRONMENT
        .iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (*name, value)))
        .collect::<Vec<_>>();

    command.env_clear();

    for (name, value) in allowed {
        if !starts_with_reserved_prefix(OsStr::new(name)) {
            command.env(name, value);
        }
    }
}

#[cfg(unix)]
fn open_exchange_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(windows)]
fn open_exchange_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt as _;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_exchange_file(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}

#[cfg(windows)]
fn starts_with_reserved_prefix(name: &OsStr) -> bool {
    name.to_string_lossy()
        .to_ascii_uppercase()
        .starts_with(RESERVED_RENDERER_ENV_PREFIX)
}

#[cfg(not(windows))]
fn starts_with_reserved_prefix(name: &OsStr) -> bool {
    name.as_encoded_bytes()
        .starts_with(RESERVED_RENDERER_ENV_PREFIX.as_bytes())
}

struct RendererDirectory {
    path: PathBuf,
    closed: bool,
}

impl RendererDirectory {
    fn create(workspace_target_directory: &Path) -> Result<Self, InvocationError> {
        let root = workspace_target_directory.join("overseerd/renderers");

        std::fs::create_dir_all(&root).map_err(InvocationError::CreateDirectory)?;

        for _ in 0..RUN_DIRECTORY_ATTEMPTS {
            let ordinal = NEXT_RUN_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!("{}-{ordinal}", std::process::id()));

            match create_private_directory(&path) {
                Ok(()) => {
                    return Ok(Self {
                        path,
                        closed: false,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(InvocationError::CreateDirectory(error)),
            }
        }

        Err(InvocationError::DirectoryExhausted)
    }

    fn close(mut self) -> std::io::Result<()> {
        std::fs::remove_dir_all(&self.path)?;
        self.closed = true;

        Ok(())
    }
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;

    let mut builder = std::fs::DirBuilder::new();

    builder.mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir(path)
}

impl Drop for RendererDirectory {
    fn drop(&mut self) {
        if !self.closed {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests;
