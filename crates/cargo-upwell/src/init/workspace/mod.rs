//! Crash-recoverable publication of parent workspace manifests.

mod platform;

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read as _, Seek as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, anyhow};
use fs2::FileExt as _;

use super::InitError;

const LOCK_NAME: &str = ".cargo-upwell-workspace.lock";
const RECOVERY_PREFIX: &str = ".cargo-upwell-workspace-recovery-";
const SNAPSHOT_SUFFIX: &str = ".snapshot";
const DISPLACED_SUFFIX: &str = ".displaced";
const CANDIDATE_SUFFIX: &str = ".candidate";
const PENDING_SUFFIX: &str = ".pending";
const COMPLETE_SUFFIX: &str = ".complete";
const INDETERMINATE_SUFFIX: &str = ".indeterminate";
const RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_UNRESOLVED_RECOVERIES: usize = 8;
const MAX_COMPLETED_RECOVERIES: usize = 8;

pub(super) fn register(project: &Path) -> Result<(), InitError> {
    register_impl(project, || {}, || {})
}

#[cfg(test)]
pub(super) fn register_with(
    project: &Path,
    before_exchange: impl FnOnce(),
    after_validation: impl FnOnce(),
) -> Result<(), InitError> {
    register_impl(project, before_exchange, after_validation)
}

fn register_impl(
    project: &Path,
    before_exchange: impl FnOnce(),
    after_validation: impl FnOnce(),
) -> Result<(), InitError> {
    let manifest = project
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("Cargo.toml");
    let parent = manifest.parent().ok_or_else(|| {
        workspace_error(
            project,
            &manifest,
            anyhow!("workspace manifest has no parent"),
        )
    })?;

    platform::ensure_supported(project, &manifest)?;
    let _sidecar_lock = lock_sidecar(parent)
        .map_err(|source| workspace_error(project, &manifest, source.into()))?;
    cleanup_recoveries(parent, SystemTime::now(), MAX_UNRESOLVED_RECOVERIES - 1)
        .map_err(|source| workspace_error(project, &manifest, source.into()))?;

    let mut active = platform::open_manifest(&manifest)
        .map_err(|source| workspace_error(project, &manifest, source.into()))?;
    #[cfg(not(windows))]
    active
        .lock_exclusive()
        .map_err(|source| workspace_error(project, &manifest, source.into()))?;

    let mut source = String::new();
    active
        .read_to_string(&mut source)
        .map_err(|source| workspace_error(project, &manifest, source.into()))?;
    let Some(replacement) = prepare_replacement(project, &manifest, &source)? else {
        return Ok(());
    };

    before_exchange();

    let mut recovery = Recovery::create(parent, &active, &source)
        .map_err(|source| workspace_error(project, &manifest, source.into()))?;
    let candidate_path = &recovery.displaced;
    let candidate = match prepare_candidate(&active, candidate_path, replacement.as_bytes()) {
        Ok(candidate) => candidate,
        Err(source) => {
            recovery.remove_pre_mutation(parent);
            return Err(workspace_error(project, &manifest, source));
        }
    };
    if let Err(source) = platform::sync_directory(parent) {
        drop(candidate);
        recovery.remove_pre_mutation(parent);
        return Err(workspace_error(project, &manifest, source.into()));
    }

    if let Err(source) = validate_unchanged(&mut active, &candidate, &manifest, source.as_bytes()) {
        drop(candidate);
        recovery.remove_pre_mutation(parent);
        return match source.downcast_ref::<ManifestChanged>() {
            Some(_) => Err(InitError::WorkspaceManifestConflict {
                project: project.to_path_buf(),
                manifest,
            }),
            None => Err(workspace_error(project, &manifest, source)),
        };
    }

    after_validation();

    match platform::exchange(&mut active, &manifest, candidate_path, &recovery.displaced) {
        Ok(()) => {}
        Err(platform::ExchangeError::Unsupported(reason)) => {
            recovery.remove_pre_mutation(parent);
            return Err(InitError::UnsupportedWorkspacePublication {
                project: project.to_path_buf(),
                manifest,
                reason,
            });
        }
        Err(platform::ExchangeError::Io(source)) => {
            recovery.remove_pre_mutation(parent);
            return Err(workspace_error(project, &manifest, source.into()));
        }
    }

    let post_result = platform::sync_directory(parent).and_then(|()| {
        platform::verify_publication(
            &mut active,
            &candidate,
            &manifest,
            &recovery.displaced,
            source.as_bytes(),
            replacement.as_bytes(),
        )
    });
    if let Err(source) = post_result {
        if !platform::path_names_file(&candidate, &manifest).unwrap_or(false) {
            let _ = platform::exchange(
                &mut active,
                &manifest,
                &recovery.displaced,
                &recovery.displaced,
            )
            .and_then(|()| platform::sync_directory(parent).map_err(platform::ExchangeError::Io));
        }
        recovery.mark_indeterminate(parent);

        return Err(InitError::WorkspacePublicationIndeterminate {
            project: project.to_path_buf(),
            manifest,
            snapshot: recovery.snapshot,
            displaced: recovery.displaced,
            source: source.into(),
        });
    }

    if let Err(source) = recovery.mark_complete(parent) {
        recovery.mark_indeterminate(parent);
        return Err(InitError::WorkspacePublicationIndeterminate {
            project: project.to_path_buf(),
            manifest,
            snapshot: recovery.snapshot,
            displaced: recovery.displaced,
            source: source.into(),
        });
    }
    Ok(())
}

fn prepare_replacement(
    project: &Path,
    manifest: &Path,
    source: &str,
) -> Result<Option<String>, InitError> {
    let mut document = source
        .parse::<toml::Table>()
        .map_err(|source| workspace_error(project, manifest, source.into()))?;
    let members = document
        .get_mut("workspace")
        .and_then(toml::Value::as_table_mut)
        .and_then(|workspace| workspace.get_mut("members"))
        .and_then(toml::Value::as_array_mut)
        .ok_or_else(|| {
            workspace_error(
                project,
                manifest,
                anyhow!("parent manifest has no workspace members array"),
            )
        })?;
    let member = project
        .file_name()
        .ok_or_else(|| {
            workspace_error(
                project,
                manifest,
                anyhow!("project path has no final component"),
            )
        })?
        .to_string_lossy()
        .into_owned();

    if members.iter().any(|value| value.as_str() == Some(&member)) {
        return Ok(None);
    }

    members.push(toml::Value::String(member));
    members.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    toml::to_string_pretty(&document)
        .map(Some)
        .map_err(|source| workspace_error(project, manifest, source.into()))
}

fn lock_sidecar(parent: &Path) -> std::io::Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(parent.join(LOCK_NAME))?;
    file.lock_exclusive()?;
    Ok(file)
}

fn prepare_candidate(active: &File, path: &Path, replacement: &[u8]) -> anyhow::Result<File> {
    let mut candidate = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("creating publication candidate `{}`", path.display()))?;
    platform::protect_recovery(active, &candidate)?;
    candidate.write_all(replacement)?;
    platform::copy_metadata(active, &candidate)?;
    platform::verify_metadata(active, &candidate)?;
    candidate.sync_all()?;
    Ok(candidate)
}

fn validate_unchanged(
    active: &mut File,
    candidate: &File,
    manifest: &Path,
    expected: &[u8],
) -> anyhow::Result<()> {
    active.rewind()?;
    let mut current = Vec::new();
    active.read_to_end(&mut current)?;
    if current != expected || !platform::path_names_file(active, manifest)? {
        return Err(ManifestChanged.into());
    }
    platform::verify_metadata(active, candidate)?;

    Ok(())
}

fn workspace_error(project: &Path, manifest: &Path, source: anyhow::Error) -> InitError {
    InitError::Workspace {
        project: project.to_path_buf(),
        manifest: manifest.to_path_buf(),
        source,
    }
}

#[derive(Debug)]
struct ManifestChanged;

impl std::fmt::Display for ManifestChanged {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("workspace manifest changed before publication")
    }
}

impl std::error::Error for ManifestChanged {}

struct Recovery {
    snapshot: PathBuf,
    displaced: PathBuf,
    pending: PathBuf,
    complete: PathBuf,
    indeterminate: PathBuf,
}

impl Recovery {
    fn create(parent: &Path, active: &File, source: &str) -> std::io::Result<Self> {
        for sequence in 0_u8..32 {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let id = format!("{timestamp:032x}-{:08x}-{sequence:02x}", std::process::id());
            let base = parent.join(format!("{RECOVERY_PREFIX}{id}"));
            let snapshot = append_suffix(&base, SNAPSHOT_SUFFIX);
            let displaced = append_suffix(&base, DISPLACED_SUFFIX);
            let pending = append_suffix(&base, PENDING_SUFFIX);
            let complete = append_suffix(&base, COMPLETE_SUFFIX);
            let indeterminate = append_suffix(&base, INDETERMINATE_SUFFIX);
            match OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .open(&snapshot)
            {
                Ok(mut file) => {
                    let initialized = (|| -> std::io::Result<()> {
                        platform::protect_recovery(active, &file)?;
                        file.write_all(source.as_bytes())?;
                        file.sync_all()?;
                        let mut state = OpenOptions::new()
                            .create_new(true)
                            .write(true)
                            .open(&pending)?;
                        state.write_all(b"pending\n")?;
                        state.sync_all()?;
                        platform::sync_directory(parent)
                    })();
                    if let Err(error) = initialized {
                        let _ = std::fs::remove_file(&snapshot);
                        let _ = std::fs::remove_file(&displaced);
                        let _ = std::fs::remove_file(&pending);
                        let _ = platform::sync_directory(parent);
                        return Err(error);
                    }
                    return Ok(Self {
                        snapshot,
                        displaced,
                        pending,
                        complete,
                        indeterminate,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate a unique workspace recovery name",
        ))
    }

    fn remove_pre_mutation(&self, parent: &Path) {
        let _ = std::fs::remove_file(&self.snapshot);
        let _ = std::fs::remove_file(&self.displaced);
        let _ = std::fs::remove_file(&self.pending);
        let _ = std::fs::remove_file(&self.complete);
        let _ = std::fs::remove_file(&self.indeterminate);
        let _ = platform::sync_directory(parent);
    }

    fn mark_complete(&mut self, parent: &Path) -> std::io::Result<()> {
        std::fs::rename(&self.pending, &self.complete)?;
        platform::sync_directory(parent)
    }

    fn mark_indeterminate(&self, parent: &Path) {
        if std::fs::rename(&self.pending, &self.indeterminate).is_err() {
            let _ = std::fs::rename(&self.complete, &self.indeterminate);
        }
        let _ = platform::sync_directory(parent);
    }
}

fn append_suffix(base: &Path, suffix: &str) -> PathBuf {
    let mut path = base.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

fn cleanup_recoveries(parent: &Path, now: SystemTime, retain: usize) -> std::io::Result<()> {
    let mut groups: BTreeMap<String, (SystemTime, Vec<PathBuf>)> = BTreeMap::new();
    for entry in std::fs::read_dir(parent)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(id) = recovery_id(&name) else {
            continue;
        };
        let modified = entry.metadata()?.modified()?;
        let group = groups
            .entry(id.to_owned())
            .or_insert_with(|| (modified, Vec::new()));
        group.0 = group.0.max(modified);
        group.1.push(entry.path());
    }

    let mut ordered: Vec<_> = groups.into_values().collect();
    ordered.sort_by_key(|(modified, _)| *modified);
    let unresolved_count = ordered
        .iter()
        .filter(|(_, paths)| {
            paths.iter().any(|path| {
                let path = path.as_os_str().to_string_lossy();
                path.ends_with(PENDING_SUFFIX) || path.ends_with(INDETERMINATE_SUFFIX)
            })
        })
        .count();
    if unresolved_count > retain {
        return Err(std::io::Error::other(format!(
            "workspace has {unresolved_count} unresolved recovery transactions; reconcile them before retrying"
        )));
    }
    let completed_count = ordered.len() - unresolved_count;
    let mut completed_to_remove = completed_count.saturating_sub(MAX_COMPLETED_RECOVERIES);

    for (modified, paths) in ordered {
        if paths.iter().any(|path| {
            let path = path.as_os_str().to_string_lossy();
            path.ends_with(PENDING_SUFFIX) || path.ends_with(INDETERMINATE_SUFFIX)
        }) {
            continue;
        }
        let expired = now
            .duration_since(modified)
            .is_ok_and(|age| age >= RETENTION);
        if !expired && completed_to_remove == 0 {
            continue;
        }
        for path in paths {
            std::fs::remove_file(path)?;
        }
        completed_to_remove = completed_to_remove.saturating_sub(1);
    }
    platform::sync_directory(parent)
}

fn recovery_id(name: &str) -> Option<&str> {
    let rest = name.strip_prefix(RECOVERY_PREFIX)?;
    for suffix in [
        SNAPSHOT_SUFFIX,
        DISPLACED_SUFFIX,
        CANDIDATE_SUFFIX,
        PENDING_SUFFIX,
        COMPLETE_SUFFIX,
        INDETERMINATE_SUFFIX,
    ] {
        if let Some(id) = rest.strip_suffix(suffix)
            && valid_recovery_id(id)
        {
            return Some(id);
        }
    }
    None
}

fn valid_recovery_id(id: &str) -> bool {
    let bytes = id.as_bytes();

    bytes.len() == 44
        && bytes[32] == b'-'
        && bytes[41] == b'-'
        && bytes[..32].iter().all(u8::is_ascii_hexdigit)
        && bytes[33..41].iter().all(u8::is_ascii_hexdigit)
        && bytes[42..].iter().all(u8::is_ascii_hexdigit)
}

#[cfg(test)]
mod tests;
