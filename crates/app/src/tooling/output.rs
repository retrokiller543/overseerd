use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use overseerd_tooling_schema::{ProbeEnvelope, TOOLING_PROBE_OUTPUT_ENV};
use thiserror::Error;

const TEMP_CREATE_ATTEMPTS: u16 = 128;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Why an invoker-selected probe response path is unsafe or unusable.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum ToolingProbeOutputTargetError {
    /// The response path does not name a file beneath a parent directory.
    #[error("the response path has no file name")]
    MissingFileName,
    /// The response path's parent does not exist.
    #[error("the response path parent does not exist")]
    MissingParent,
    /// The response path's parent is not a real directory.
    #[error("the response path parent is not a regular directory")]
    InvalidParent,
    /// The final response path is an existing symbolic link.
    #[error("the response path is an existing symbolic link")]
    ExistingSymlink,
    /// The final response path is an existing regular file.
    #[error("the response path already exists")]
    ExistingFile,
    /// The final response path is an existing non-regular file.
    #[error("the response path is an existing non-regular file")]
    ExistingNonRegular,
}

/// A typed failure while atomically publishing a completed probe response file.
#[derive(Error)]
#[non_exhaustive]
pub enum ToolingProbeOutputError {
    /// The required response file environment variable is absent.
    #[error("required tooling probe environment variable '{TOOLING_PROBE_OUTPUT_ENV}' is not set")]
    MissingOutputPath,
    /// The envelope could not be validated or serialized before file-system work began.
    #[error("failed to validate or serialize the tooling probe envelope")]
    Serialize(#[source] overseerd_tooling_schema::ProbeEmitError),
    /// The invoker-selected final response target violates the output contract.
    #[error("invalid tooling probe response target: {reason}")]
    InvalidTarget {
        /// Invoker-selected final response path.
        path: PathBuf,
        /// Structural target failure.
        reason: ToolingProbeOutputTargetError,
    },
    /// Every bounded unique sibling temporary name already existed.
    #[error("exhausted private tooling probe temporary names after {attempts} attempts")]
    TempExhausted {
        /// Invoker-selected final response path.
        path: PathBuf,
        /// Number of exclusive creation attempts.
        attempts: u16,
    },
    /// A private sibling temporary file could not be created exclusively.
    #[error("failed to create a private temporary tooling response")]
    Create {
        /// Invoker-selected final response path.
        path: PathBuf,
        /// File-system failure.
        #[source]
        source: std::io::Error,
    },
    /// The complete serialized response could not be written or flushed.
    #[error("failed to write the temporary tooling response")]
    Write {
        /// Invoker-selected final response path.
        path: PathBuf,
        /// File-system failure.
        #[source]
        source: std::io::Error,
    },
    /// The temporary response or published parent directory could not be synchronized.
    #[error("failed to synchronize the tooling response")]
    Sync {
        /// Invoker-selected final response path.
        path: PathBuf,
        /// File-system failure.
        #[source]
        source: std::io::Error,
    },
    /// The completed sibling temporary file could not be atomically published without replacement.
    #[error("failed to publish the tooling probe response")]
    Publish {
        /// Invoker-selected final response path.
        path: PathBuf,
        /// File-system failure.
        #[source]
        source: std::io::Error,
    },
    /// The temporary name could not be removed after the completed response was published.
    #[error("failed to remove the published tooling response temporary name")]
    Cleanup {
        /// Invoker-selected final response path.
        path: PathBuf,
        /// File-system failure.
        #[source]
        source: std::io::Error,
    },
}

impl std::fmt::Debug for ToolingProbeOutputError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingOutputPath => formatter.write_str("MissingOutputPath"),
            Self::Serialize(_) => formatter.write_str("Serialize"),
            Self::InvalidTarget { reason, .. } => formatter
                .debug_tuple("InvalidTarget")
                .field(reason)
                .finish(),
            Self::TempExhausted { attempts, .. } => formatter
                .debug_struct("TempExhausted")
                .field("attempts", attempts)
                .finish(),
            Self::Create { source, .. } => format_io_kind(formatter, "Create", source.kind()),
            Self::Write { source, .. } => format_io_kind(formatter, "Write", source.kind()),
            Self::Sync { source, .. } => format_io_kind(formatter, "Sync", source.kind()),
            Self::Publish { source, .. } => format_io_kind(formatter, "Publish", source.kind()),
            Self::Cleanup { source, .. } => format_io_kind(formatter, "Cleanup", source.kind()),
        }
    }
}

fn format_io_kind(
    formatter: &mut std::fmt::Formatter<'_>,
    variant: &str,
    kind: std::io::ErrorKind,
) -> std::fmt::Result {
    formatter
        .debug_struct(variant)
        .field("kind", &kind)
        .finish()
}

/// Emits one validated canonical probe envelope to the invoker-selected response path.
///
/// The #152 invoker contract supplies `TOOLING_PROBE_OUTPUT_ENV` as a non-existing file path in an
/// existing private directory. JSON is never written to stdout.
pub fn emit_probe_envelope_from_env(
    envelope: &ProbeEnvelope,
) -> Result<(), ToolingProbeOutputError> {
    let path = std::env::var_os(TOOLING_PROBE_OUTPUT_ENV)
        .map(PathBuf::from)
        .ok_or(ToolingProbeOutputError::MissingOutputPath)?;

    emit_probe_envelope(path, envelope)
}

/// Privately and atomically publishes one validated canonical probe envelope.
///
/// Validation and serialization complete before the target is inspected. The final path must not
/// exist and its parent must already be a real directory. A unique sibling is created exclusively,
/// restricted to mode `0o600` on Unix, fully written, flushed, and synchronized before one atomic
/// no-clobber publication. Publication atomically fails if any actor creates the final path first,
/// so partial JSON and replacement are impossible. #152 must still supply a private directory to
/// isolate the response and its sibling temporary name from other users, not to make publication
/// correct. A crash can leave both names, but both refer to the same completed response inode and
/// the future #152 private-directory cleanup removes the temporary name.
pub fn emit_probe_envelope(
    path: impl AsRef<Path>,
    envelope: &ProbeEnvelope,
) -> Result<(), ToolingProbeOutputError> {
    emit_probe_envelope_impl(path.as_ref(), envelope, |_| {})
}

fn emit_probe_envelope_impl(
    path: &Path,
    envelope: &ProbeEnvelope,
    before_publish: impl FnOnce(&Path),
) -> Result<(), ToolingProbeOutputError> {
    let path = path.to_path_buf();
    let mut bytes = envelope
        .to_json()
        .map_err(ToolingProbeOutputError::Serialize)?
        .into_bytes();
    let parent = validate_target(&path)?;
    let mut temporary = TemporaryResponse::create(&path, &parent)?;

    bytes.push(b'\n');
    temporary
        .file
        .as_mut()
        .expect("temporary response remains open while writing")
        .write_all(&bytes)
        .and_then(|()| {
            temporary
                .file
                .as_mut()
                .expect("temporary response remains open while flushing")
                .flush()
        })
        .map_err(|source| ToolingProbeOutputError::Write {
            path: path.clone(),
            source,
        })?;
    temporary
        .file
        .as_ref()
        .expect("temporary response remains open while synchronizing")
        .sync_all()
        .map_err(|source| ToolingProbeOutputError::Sync {
            path: path.clone(),
            source,
        })?;
    temporary.file.take();

    before_publish(&path);
    platform::publish_no_replace(&temporary.path, &path).map_err(|source| {
        ToolingProbeOutputError::Publish {
            path: path.clone(),
            source,
        }
    })?;

    platform::sync_parent(&parent).map_err(|source| ToolingProbeOutputError::Sync {
        path: path.clone(),
        source,
    })?;
    temporary
        .remove()
        .map_err(|source| ToolingProbeOutputError::Cleanup {
            path: path.clone(),
            source,
        })?;

    platform::sync_parent(&parent).map_err(|source| ToolingProbeOutputError::Sync {
        path: path.clone(),
        source,
    })?;

    Ok(())
}

#[cfg(test)]
fn emit_probe_envelope_with_hook(
    path: impl AsRef<Path>,
    envelope: &ProbeEnvelope,
    before_publish: impl FnOnce(&Path),
) -> Result<(), ToolingProbeOutputError> {
    emit_probe_envelope_impl(path.as_ref(), envelope, before_publish)
}

fn validate_target(path: &Path) -> Result<PathBuf, ToolingProbeOutputError> {
    if path.file_name().is_none() {
        return Err(invalid_target(
            path,
            ToolingProbeOutputTargetError::MissingFileName,
        ));
    }

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let metadata = match std::fs::symlink_metadata(&parent) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(invalid_target(
                path,
                ToolingProbeOutputTargetError::MissingParent,
            ));
        }
        Err(_) => {
            return Err(invalid_target(
                path,
                ToolingProbeOutputTargetError::InvalidParent,
            ));
        }
    };

    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid_target(
            path,
            ToolingProbeOutputTargetError::InvalidParent,
        ));
    }

    validate_final_absent(path)?;

    Ok(parent)
}

fn validate_final_absent(path: &Path) -> Result<(), ToolingProbeOutputError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            return Err(invalid_target(
                path,
                ToolingProbeOutputTargetError::ExistingNonRegular,
            ));
        }
    };
    let reason = if metadata.file_type().is_symlink() {
        ToolingProbeOutputTargetError::ExistingSymlink
    } else if metadata.is_file() {
        ToolingProbeOutputTargetError::ExistingFile
    } else {
        ToolingProbeOutputTargetError::ExistingNonRegular
    };

    Err(invalid_target(path, reason))
}

fn invalid_target(path: &Path, reason: ToolingProbeOutputTargetError) -> ToolingProbeOutputError {
    ToolingProbeOutputError::InvalidTarget {
        path: path.to_path_buf(),
        reason,
    }
}

/// An exclusively created sibling file removed automatically unless cleanup already succeeded.
struct TemporaryResponse {
    path: PathBuf,
    file: Option<File>,
    removed: bool,
}

impl TemporaryResponse {
    fn create(final_path: &Path, parent: &Path) -> Result<Self, ToolingProbeOutputError> {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);

        for attempt in 0..TEMP_CREATE_ATTEMPTS {
            let path = parent.join(format!(
                ".overseerd-tooling-probe-{}-{sequence}-{attempt}.tmp",
                std::process::id()
            ));

            match platform::create_private(&path) {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file: Some(file),
                        removed: false,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(source) => {
                    return Err(ToolingProbeOutputError::Create {
                        path: final_path.to_path_buf(),
                        source,
                    });
                }
            }
        }

        Err(ToolingProbeOutputError::TempExhausted {
            path: final_path.to_path_buf(),
            attempts: TEMP_CREATE_ATTEMPTS,
        })
    }

    fn remove(&mut self) -> std::io::Result<()> {
        std::fs::remove_file(&self.path)?;
        self.removed = true;

        Ok(())
    }
}

impl Drop for TemporaryResponse {
    fn drop(&mut self) {
        if !self.removed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

mod platform;

#[cfg(test)]
mod tests;
