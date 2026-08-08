//! Private, bounded completion snapshots refreshed by explicit tooling probes.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use upwell_tooling_schema::{CliMetadata, ProbeOutcome, ResourceKind, ToolingDocument};

use crate::{FeatureSelection, ToolingProbe};

const SNAPSHOT_SCHEMA: u16 = 1;
const MAX_CACHE_BYTES: u64 = 1024 * 1024;
const MAX_CANDIDATES: usize = 4096;
const MAX_VALUE_BYTES: usize = 512;
const MAX_HELP_BYTES: usize = 1024;
const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const MAX_CLOCK_SKEW: Duration = Duration::from_secs(5 * 60);

/// Semantic completion namespace understood by cargo-upwell's command tree.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CandidateKind {
    Package,
    Binary,
    Feature,
    Resource,
    Contributor,
    Plugin,
    Scope,
    Facet,
    Template,
}

impl CandidateKind {
    const fn key(self) -> &'static str {
        match self {
            Self::Package => "package",
            Self::Binary => "binary",
            Self::Feature => "feature",
            Self::Resource => "resource",
            Self::Contributor => "contributor",
            Self::Plugin => "plugin",
            Self::Scope => "scope",
            Self::Facet => "facet",
            Self::Template => "template",
        }
    }
}

/// One shell-safe cached completion candidate.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Candidate {
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Snapshot {
    schema: u16,
    cargo_upwell_version: String,
    generated_at: u64,
    workspace_root: PathBuf,
    package_id: String,
    binary: String,
    no_default_features: bool,
    all_features: bool,
    features: Vec<String>,
    target: Option<String>,
    candidates: BTreeMap<String, Vec<Candidate>>,
}

/// Best-effort cache failure. Tooling commands remain successful when cache refresh fails.
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("the platform completion cache directory is unavailable")]
    Unavailable,
    #[error("the application tooling probe did not produce a successful document")]
    ProbeFailed,
    #[error("the completion snapshot exceeds the {MAX_CACHE_BYTES}-byte cache limit")]
    TooLarge,
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Refreshes the active workspace's completion snapshot after a successful explicit probe.
pub fn refresh(
    probe: &ToolingProbe,
    features: &FeatureSelection,
    target: Option<&str>,
) -> Result<(), CacheError> {
    let document = match &probe.probe.envelope.outcome {
        ProbeOutcome::Success { document } => document,
        ProbeOutcome::Failure { .. } => return Err(CacheError::ProbeFailed),
    };
    let root = cache_root()?.join(workspace_key(&probe.workspace.workspace_root));

    create_private_directory(&root)?;
    let snapshot = Snapshot {
        schema: SNAPSHOT_SCHEMA,
        cargo_upwell_version: env!("CARGO_PKG_VERSION").to_owned(),
        generated_at: unix_seconds(SystemTime::now()),
        workspace_root: canonical_or_original(&probe.workspace.workspace_root),
        package_id: probe.target.package_id.clone(),
        binary: probe.target.binary_name.clone(),
        no_default_features: features.no_default_features,
        all_features: features.all_features,
        features: features.normalized_features(),
        target: target.map(str::to_owned),
        candidates: project_candidates(probe, document),
    };
    let mut serialized = serde_json::to_vec(&snapshot)?;

    serialized.push(b'\n');
    if serialized.len() as u64 > MAX_CACHE_BYTES {
        return Err(CacheError::TooLarge);
    }
    let mut temporary = NamedTempFile::new_in(&root)?;

    set_private_file_permissions(temporary.as_file())?;
    temporary.write_all(&serialized)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(root.join("active.json"))
        .map_err(|error| error.error)?;

    Ok(())
}

/// Reads cached candidates for the active workspace without executing Cargo or application code.
pub fn candidates(kind: CandidateKind, current_dir: &Path) -> Vec<Candidate> {
    read_candidates(kind, current_dir).unwrap_or_default()
}

fn read_candidates(kind: CandidateKind, current_dir: &Path) -> Result<Vec<Candidate>, CacheError> {
    let root = cache_root()?;
    read_candidates_from(kind, current_dir, &root)
}

fn read_candidates_from(
    kind: CandidateKind,
    current_dir: &Path,
    root: &Path,
) -> Result<Vec<Candidate>, CacheError> {
    reject_symlink(root)?;
    let workspace = active_workspace(current_dir).ok_or(CacheError::Unavailable)?;
    let directory = root.join(workspace_key(&workspace));
    let path = directory.join("active.json");

    if !path.is_file() {
        return Err(CacheError::Unavailable);
    }
    reject_symlink(&directory)?;
    let metadata = std::fs::symlink_metadata(&path)?;

    if metadata.file_type().is_symlink() || metadata.len() > MAX_CACHE_BYTES {
        return Ok(Vec::new());
    }
    let mut source = String::new();

    let file = open_cache_file(&path)?;
    let metadata = file.metadata()?;

    if metadata.len() > MAX_CACHE_BYTES {
        return Ok(Vec::new());
    }
    file.take(MAX_CACHE_BYTES + 1).read_to_string(&mut source)?;
    let snapshot = serde_json::from_str::<Snapshot>(&source)?;
    let canonical_workspace = canonical_or_original(&workspace);

    let now = unix_seconds(SystemTime::now());
    if snapshot.schema != SNAPSHOT_SCHEMA
        || snapshot.cargo_upwell_version != env!("CARGO_PKG_VERSION")
        || snapshot.workspace_root != canonical_workspace
        || snapshot.generated_at > now.saturating_add(MAX_CLOCK_SKEW.as_secs())
        || now.saturating_sub(snapshot.generated_at) > MAX_AGE.as_secs()
    {
        return Ok(Vec::new());
    }
    if !snapshot_candidates_are_valid(&snapshot) {
        return Ok(Vec::new());
    }

    Ok(snapshot
        .candidates
        .get(kind.key())
        .cloned()
        .unwrap_or_default())
}

fn project_candidates(
    probe: &ToolingProbe,
    document: &ToolingDocument,
) -> BTreeMap<String, Vec<Candidate>> {
    let mut values = BTreeMap::<String, BTreeMap<String, Option<String>>>::new();

    for package in &probe.workspace.packages {
        insert(
            &mut values,
            CandidateKind::Package,
            &package.name,
            Some(&package.version),
        );
        if package.id == probe.target.package_id {
            for binary in &package.binaries {
                insert(
                    &mut values,
                    CandidateKind::Binary,
                    &binary.name,
                    Some(&package.name),
                );
            }
        }
    }
    if let Ok(metadata) = cargo_metadata::MetadataCommand::new()
        .manifest_path(probe.workspace.workspace_root.join("Cargo.toml"))
        .no_deps()
        .exec()
    {
        for package in metadata
            .packages
            .into_iter()
            .filter(|package| package.id.to_string() == probe.target.package_id)
        {
            for feature in package.features.keys() {
                insert(
                    &mut values,
                    CandidateKind::Feature,
                    feature,
                    Some(&package.name),
                );
            }
        }
    }
    if let Ok(catalog) = crate::Catalog::load(None) {
        for template in catalog.templates() {
            insert(
                &mut values,
                CandidateKind::Template,
                template.id(),
                Some(template.description()),
            );
        }
    }
    for resource in &document.resources {
        insert(
            &mut values,
            CandidateKind::Resource,
            &resource.id,
            Some(&resource.name),
        );
        match resource.kind {
            ResourceKind::Application
            | ResourceKind::Protocol
            | ResourceKind::Plugin
            | ResourceKind::Contributor => {
                insert(
                    &mut values,
                    CandidateKind::Contributor,
                    &resource.id,
                    Some(&resource.name),
                );
            }
            _ => {}
        }
        if resource.kind == ResourceKind::Plugin {
            insert(
                &mut values,
                CandidateKind::Plugin,
                &resource.id,
                Some(&resource.name),
            );
        }
        if resource.kind == ResourceKind::Scope {
            insert(
                &mut values,
                CandidateKind::Scope,
                &resource.id,
                Some(&resource.name),
            );
            if let Some(scope) = resource.labels.get("scope-id") {
                insert(
                    &mut values,
                    CandidateKind::Scope,
                    scope,
                    Some(&resource.name),
                );
            }
        }
        for facet in resource.facets.keys() {
            insert(&mut values, CandidateKind::Facet, facet, None);
        }
    }
    for facet in document.facets.keys() {
        insert(&mut values, CandidateKind::Facet, facet, None);
    }
    project_cli_candidates(document.cli.as_ref(), &mut values);

    let mut remaining = MAX_CANDIDATES;

    values
        .into_iter()
        .map(|(kind, candidates)| {
            let candidates = candidates
                .into_iter()
                .take(remaining)
                .map(|(value, help)| Candidate { value, help })
                .collect::<Vec<_>>();

            remaining = remaining.saturating_sub(candidates.len());

            (kind, candidates)
        })
        .collect()
}

fn project_cli_candidates(
    cli: Option<&CliMetadata>,
    values: &mut BTreeMap<String, BTreeMap<String, Option<String>>>,
) {
    let Some(cli) = cli else { return };

    for provider in &cli.providers {
        insert(
            values,
            CandidateKind::Resource,
            &provider.id,
            Some("CLI provider"),
        );
        insert(
            values,
            CandidateKind::Contributor,
            &provider.contributor,
            Some("CLI contributor"),
        );
    }
}

fn insert(
    values: &mut BTreeMap<String, BTreeMap<String, Option<String>>>,
    kind: CandidateKind,
    value: &str,
    help: Option<&str>,
) {
    if values.values().map(BTreeMap::len).sum::<usize>() >= MAX_CANDIDATES {
        return;
    }
    if !safe_text(value, MAX_VALUE_BYTES) {
        return;
    }
    let help = help
        .filter(|help| safe_text(help, MAX_HELP_BYTES))
        .map(str::to_owned);

    values
        .entry(kind.key().to_owned())
        .or_default()
        .entry(value.to_owned())
        .or_insert(help);
}

fn snapshot_candidates_are_valid(snapshot: &Snapshot) -> bool {
    snapshot.candidates.values().map(Vec::len).sum::<usize>() <= MAX_CANDIDATES
        && snapshot.candidates.values().flatten().all(|candidate| {
            safe_text(&candidate.value, MAX_VALUE_BYTES)
                && candidate
                    .help
                    .as_deref()
                    .is_none_or(|help| safe_text(help, MAX_HELP_BYTES))
        })
}

fn safe_text(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && !value
            .chars()
            .any(|character| character == '\0' || character.is_control())
}

fn cache_root() -> Result<PathBuf, CacheError> {
    if let Some(path) = std::env::var_os("UPWELL_COMPLETION_CACHE_DIR") {
        let path = PathBuf::from(path);

        if path.is_absolute() {
            return Ok(path);
        }
    }

    ProjectDirs::from("org", "upwell-rs", "Upwell")
        .map(|directories| directories.cache_dir().join("completions/v1"))
        .ok_or(CacheError::Unavailable)
}

fn workspace_key(workspace: &Path) -> String {
    let mut hash = 0xcbf29ce484222325_u64;

    for byte in canonical_or_original(workspace)
        .as_os_str()
        .to_string_lossy()
        .bytes()
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }

    format!("{hash:016x}")
}

fn active_workspace(start: &Path) -> Option<PathBuf> {
    let manifests = start
        .ancestors()
        .filter(|directory| directory.join("Cargo.toml").is_file())
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();

    let package = manifests.first()?.clone();
    let explicit_workspace = package_workspace(&package.join("Cargo.toml"))
        .map(|workspace| canonical_or_original(&package.join(workspace)));

    if let Some(workspace) = explicit_workspace {
        return workspace.join("Cargo.toml").is_file().then_some(workspace);
    }

    manifests
        .iter()
        .skip(1)
        .find(|workspace| workspace_includes_package(workspace, &package))
        .cloned()
        .or(Some(package))
}

fn package_workspace(path: &Path) -> Option<PathBuf> {
    read_manifest(path)?
        .get("package")?
        .get("workspace")?
        .as_str()
        .map(PathBuf::from)
}

fn workspace_includes_package(workspace: &Path, package: &Path) -> bool {
    let manifest = match read_manifest(&workspace.join("Cargo.toml")) {
        Some(manifest) => manifest,
        None => return false,
    };
    let Some(table) = manifest.get("workspace").and_then(toml::Value::as_table) else {
        return false;
    };
    let Ok(relative) = package.strip_prefix(workspace) else {
        return false;
    };
    let relative = relative.to_string_lossy();
    let excluded = table
        .get("exclude")
        .and_then(toml::Value::as_array)
        .is_some_and(|patterns| {
            patterns
                .iter()
                .filter_map(toml::Value::as_str)
                .any(|pattern| path_pattern_matches(pattern, &relative))
        });

    if excluded {
        return false;
    }

    table
        .get("members")
        .and_then(toml::Value::as_array)
        .is_some_and(|patterns| {
            patterns
                .iter()
                .filter_map(toml::Value::as_str)
                .any(|pattern| path_pattern_matches(pattern, &relative))
        })
}

fn read_manifest(path: &Path) -> Option<toml::Value> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return None;
    };
    if !metadata.is_file() || metadata.len() > MAX_CACHE_BYTES {
        return None;
    }
    let Ok(file) = OpenOptions::new().read(true).open(path) else {
        return None;
    };
    let mut source = String::new();

    if file
        .take(MAX_CACHE_BYTES + 1)
        .read_to_string(&mut source)
        .is_err()
    {
        return None;
    }

    toml::from_str(&source).ok()
}

fn path_pattern_matches(pattern: &str, relative: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix("/*") {
        relative
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/') && !suffix[1..].contains('/'))
    } else if let Some(prefix) = pattern.strip_suffix("/**") {
        relative == prefix || relative.starts_with(&format!("{prefix}/"))
    } else {
        relative == pattern
    }
}

fn canonical_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn create_private_directory(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        reject_symlink(parent)?;
    }
    match std::fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => reject_symlink(path)?,
        Err(error) => return Err(error),
    }
    set_private_directory_permissions(path)
}

fn reject_symlink(path: &Path) -> io::Result<()> {
    if is_link_like(&std::fs::symlink_metadata(path)?) {
        Err(io::Error::other(format!(
            "completion cache path `{}` is a symlink",
            path.display()
        )))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn is_link_like(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn is_link_like(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(any(unix, windows)))]
fn is_link_like(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(unix)]
fn open_cache_file(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(windows)]
fn open_cache_file(path: &Path) -> io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt as _;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_cache_file(path: &Path) -> io::Result<std::fs::File> {
    OpenOptions::new().read(true).open(path)
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file_permissions(file: &std::fs::File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    file.set_permissions(std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_private_file_permissions(_file: &std::fs::File) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests;
