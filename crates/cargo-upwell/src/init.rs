//! Catalog-backed project generation through cargo-generate.

mod catalog;
mod publish;

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use cargo_generate::{GenerateArgs, TemplatePath, Vcs};
use fs2::FileExt as _;
use tempfile::TempDir;
use thiserror::Error;

pub use catalog::{
    Catalog, CatalogError, GitReference, TemplateEntry, TemplateSource, ToolEntry,
    default_catalog_path,
};

/// Template source selected for one initialization.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TemplateSelection {
    /// A built-in or user catalog template. `None` is resolved by the CLI selector.
    Catalog {
        /// Exact catalog template ID.
        template: Option<String>,
        /// Explicit catalog file. Requires an explicit template ID.
        catalog_path: Option<PathBuf>,
    },
    /// A direct local cargo-generate template directory.
    Local(PathBuf),
}

/// Request to generate one project from an Upwell catalog template.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitRequest {
    /// Exact project directory to populate.
    pub destination: PathBuf,
    /// Cargo package/project name. Defaults to the destination directory name.
    pub name: Option<String>,
    /// Built-in, catalog, or direct local template selection.
    pub template: TemplateSelection,
    /// Add the generated crate to an immediate parent Cargo workspace.
    pub workspace: bool,
    /// Skip creation of a Git repository.
    pub no_vcs: bool,
    /// Additional cargo-generate template values in `key=value` form.
    pub define: Vec<String>,
    /// Override the generated Upwell dependency with this local repository path.
    pub upwell_path: Option<PathBuf>,
}

/// Completed project generation details.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitResult {
    /// Generated project directory.
    pub path: PathBuf,
    /// Selected catalog template ID, or `local` for a direct path.
    pub template: String,
}

/// Failure to resolve or generate an Upwell project template.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum InitError {
    /// The destination has no usable project name and none was supplied.
    #[error("a project name is required when the destination has no final path component")]
    MissingName,
    /// A reserved built-in template variable was supplied by the caller.
    #[error("template value `{0}` is reserved by cargo upwell")]
    ReservedValue(String),
    /// The selected template path does not exist or is not a directory.
    #[error("template path `{0}` is not a directory")]
    InvalidTemplatePath(PathBuf),
    /// The exact destination already exists.
    #[error("destination `{0}` already exists")]
    DestinationExists(PathBuf),
    /// No catalog template was selected.
    #[error("a template ID is required")]
    MissingTemplate,
    /// The exact destination could not be created.
    #[error("failed to create destination `{path}`")]
    CreateDestination {
        /// Requested project directory.
        path: PathBuf,
        /// Filesystem failure.
        #[source]
        source: std::io::Error,
    },
    /// The generated project could not be registered with its immediate parent workspace.
    #[error("failed to add `{project}` to workspace `{manifest}`")]
    Workspace {
        /// Generated project directory.
        project: PathBuf,
        /// Parent workspace manifest.
        manifest: PathBuf,
        /// Manifest read, parse, validation, or write failure.
        #[source]
        source: anyhow::Error,
    },
    /// The parent workspace manifest changed while this command was preparing an update.
    #[error(
        "workspace manifest `{manifest}` changed or is being updated; no workspace changes were written; reconcile the manifest and retry `cargo upwell init`"
    )]
    WorkspaceManifestConflict {
        /// Generated project directory that was being registered.
        project: PathBuf,
        /// Parent workspace manifest that could not safely be replaced.
        manifest: PathBuf,
    },
    /// Catalog loading or template selection failed.
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    /// A built-in template could not be materialized.
    #[error("failed to prepare built-in template `{template}`")]
    BuiltinTemplate {
        /// Built-in template ID.
        template: String,
        /// Filesystem failure.
        #[source]
        source: std::io::Error,
    },
    /// cargo-generate rejected or failed to expand the selected template.
    #[error("cargo-generate failed for template `{template}`: {source}")]
    Generate {
        /// Selected template ID.
        template: String,
        /// Generator failure.
        #[source]
        source: anyhow::Error,
    },
}

/// Generates one Upwell project using cargo-generate.
pub fn init_project(request: InitRequest) -> Result<InitResult, InitError> {
    let name = request
        .name
        .clone()
        .or_else(|| destination_name(&request.destination))
        .ok_or(InitError::MissingName)?;
    let (template_id, template_path) = resolve_template(&request.template)?;

    let config = empty_generator_config().map_err(|source| InitError::BuiltinTemplate {
        template: template_id.clone(),
        source,
    })?;
    let mut define = request.define;

    reject_reserved_values(&define)?;
    define.push(format!("upwell_version={}", env!("CARGO_PKG_VERSION")));
    let (upwell_dependency, upwell_app_dependency) =
        upwell_dependencies(request.upwell_path.as_deref()).map_err(|source| {
            InitError::BuiltinTemplate {
                template: template_id.clone(),
                source,
            }
        })?;

    define.push(format!("upwell_dependency={upwell_dependency}"));
    define.push(format!("upwell_app_dependency={upwell_app_dependency}"));
    define.push(format!(
        "upwell_base_dependency={}",
        upwell_base_dependency(request.upwell_path.as_deref()).map_err(|source| {
            InitError::BuiltinTemplate {
                template: template_id.clone(),
                source,
            }
        })?
    ));
    define.push(format!(
        "upwell_macros_core_dependency={}",
        upwell_macros_core_dependency(request.upwell_path.as_deref()).map_err(|source| {
            InitError::BuiltinTemplate {
                template: template_id.clone(),
                source,
            }
        })?
    ));

    let arguments = GenerateArgs {
        template_path,
        name: Some(name),
        force: true,
        config: Some(config.path().join("config.toml")),
        vcs: Some(if request.no_vcs { Vcs::None } else { Vcs::Git }),
        lib: false,
        bin: true,
        define,
        init: true,
        destination: Some(request.destination.clone()),
        force_git_init: !request.no_vcs && !request.workspace,
        allow_commands: false,
        overwrite: false,
        no_workspace: true,
        ..GenerateArgs::default()
    };

    let staging = create_staging_destination(&request.destination)?;
    let generated = cargo_generate::generate(GenerateArgs {
        destination: Some(staging.path().to_path_buf()),
        ..arguments
    })
    .map_err(|source| InitError::Generate {
        template: template_id.clone(),
        source,
    })?;
    debug_assert_eq!(generated, staging.path());
    let path = publish::commit(staging, &request.destination)?;
    if request.workspace
        && let Err(error) = add_to_parent_workspace(&path)
    {
        let _ = std::fs::remove_dir_all(&path);

        return Err(error);
    }

    Ok(InitResult {
        path,
        template: template_id,
    })
}

fn resolve_template(selection: &TemplateSelection) -> Result<(String, TemplatePath), InitError> {
    match selection {
        TemplateSelection::Local(path) => {
            validate_template_path(path)?;

            Ok((
                String::from("local"),
                TemplatePath {
                    path: Some(path.to_string_lossy().into_owned()),
                    ..TemplatePath::default()
                },
            ))
        }
        TemplateSelection::Catalog {
            template,
            catalog_path,
        } => {
            let id = template.as_deref().ok_or(InitError::MissingTemplate)?;
            let catalog = Catalog::load(catalog_path.as_deref())?;
            let template = catalog.template(id)?;

            match template.source() {
                TemplateSource::Local(path) => {
                    validate_template_path(path)?;
                    Ok((
                        id.to_owned(),
                        TemplatePath {
                            path: Some(path.to_string_lossy().into_owned()),
                            ..TemplatePath::default()
                        },
                    ))
                }
                TemplateSource::Git {
                    repository,
                    reference,
                } => {
                    let mut path = TemplatePath {
                        git: Some(repository.clone()),
                        ..TemplatePath::default()
                    };
                    match reference {
                        Some(GitReference::Branch(branch)) => path.branch = Some(branch.clone()),
                        Some(GitReference::Tag(tag)) => path.tag = Some(tag.clone()),
                        Some(GitReference::Revision(revision)) => {
                            path.revision = Some(revision.clone());
                        }
                        None => {}
                    }

                    Ok((id.to_owned(), path))
                }
            }
        }
    }
}

fn destination_name(destination: &Path) -> Option<String> {
    destination
        .file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
}

fn validate_template_path(path: &Path) -> Result<(), InitError> {
    if path.is_dir() {
        Ok(())
    } else {
        Err(InitError::InvalidTemplatePath(path.to_path_buf()))
    }
}

fn create_staging_destination(path: &Path) -> Result<TempDir, InitError> {
    if path.exists() {
        return Err(InitError::DestinationExists(path.to_path_buf()));
    }

    let parent = path.parent().unwrap_or_else(|| Path::new("."));

    tempfile::Builder::new()
        .prefix(".cargo-upwell-")
        .tempdir_in(parent)
        .map_err(|source| InitError::CreateDestination {
            path: path.to_path_buf(),
            source,
        })
}

fn reject_reserved_values(values: &[String]) -> Result<(), InitError> {
    for value in values {
        let name = value
            .split_once('=')
            .map_or(value.as_str(), |(name, _)| name);

        if matches!(
            name,
            "upwell_version"
                | "upwell_dependency"
                | "upwell_app_dependency"
                | "upwell_base_dependency"
                | "upwell_macros_core_dependency"
        ) {
            return Err(InitError::ReservedValue(name.to_owned()));
        }
    }

    Ok(())
}

fn empty_generator_config() -> Result<TempDir, std::io::Error> {
    let directory = tempfile::Builder::new()
        .prefix("cargo-upwell-config-")
        .tempdir()?;

    std::fs::write(directory.path().join("config.toml"), "")?;

    Ok(directory)
}

fn absolute_path(path: &Path) -> Result<PathBuf, std::io::Error> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn upwell_dependencies(path: Option<&Path>) -> Result<(String, String), std::io::Error> {
    match path {
        Some(path) => {
            let path = absolute_path(path)?;
            let facade = toml::Value::String(path.to_string_lossy().into_owned()).to_string();
            let app = toml::Value::String(path.join("crates/app").to_string_lossy().into_owned())
                .to_string();

            Ok((
                format!(
                    "{{ path = {facade}, default-features = false, features = [\"tooling\"] }}"
                ),
                format!("{{ path = {app}, default-features = false }}"),
            ))
        }
        None => Ok((
            format!(
                "{{ version = \"={}\", default-features = false, features = [\"tooling\"] }}",
                env!("CARGO_PKG_VERSION")
            ),
            format!(
                "{{ version = \"={}\", default-features = false }}",
                env!("CARGO_PKG_VERSION")
            ),
        )),
    }
}

fn upwell_macros_core_dependency(path: Option<&Path>) -> Result<String, std::io::Error> {
    match path {
        Some(path) => {
            let path = absolute_path(path)?.join("crates/macros-core");
            let path = toml::Value::String(path.to_string_lossy().into_owned()).to_string();

            Ok(format!("{{ path = {path}, default-features = false }}"))
        }
        None => Ok(format!(
            "{{ version = \"={}\", default-features = false }}",
            env!("CARGO_PKG_VERSION")
        )),
    }
}

fn upwell_base_dependency(path: Option<&Path>) -> Result<String, std::io::Error> {
    match path {
        Some(path) => {
            let path = absolute_path(path)?;
            let path = toml::Value::String(path.to_string_lossy().into_owned()).to_string();

            Ok(format!("{{ path = {path}, default-features = false }}"))
        }
        None => Ok(format!(
            "{{ version = \"={}\", default-features = false }}",
            env!("CARGO_PKG_VERSION")
        )),
    }
}

fn add_to_parent_workspace(project: &Path) -> Result<(), InitError> {
    add_to_parent_workspace_with(project, || {})
}

fn add_to_parent_workspace_with(
    project: &Path,
    before_replace: impl FnOnce(),
) -> Result<(), InitError> {
    let manifest = project
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("Cargo.toml");
    let parent = manifest.parent().ok_or_else(|| InitError::Workspace {
        project: project.to_path_buf(),
        manifest: manifest.clone(),
        source: anyhow::anyhow!("workspace manifest has no parent"),
    })?;
    let lock_path = parent.join(".cargo-upwell-workspace.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .and_then(|file| {
            file.lock_exclusive()?;
            Ok(file)
        })
        .map_err(|source| InitError::Workspace {
            project: project.to_path_buf(),
            manifest: manifest.clone(),
            source: source.into(),
        })?;
    let mut manifest_file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&manifest)
        .and_then(|file| {
            file.lock_exclusive()?;
            Ok(file)
        })
        .map_err(|source| InitError::Workspace {
            project: project.to_path_buf(),
            manifest: manifest.clone(),
            source: source.into(),
        })?;
    let result = (|| -> anyhow::Result<bool> {
        let mut source = String::new();

        std::io::Read::read_to_string(&mut manifest_file, &mut source)?;
        let mut document = source.parse::<toml::Table>()?;
        let members = document
            .get_mut("workspace")
            .and_then(toml::Value::as_table_mut)
            .and_then(|workspace| workspace.get_mut("members"))
            .and_then(toml::Value::as_array_mut)
            .ok_or_else(|| anyhow::anyhow!("parent manifest has no workspace members array"))?;
        let member = project
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("project path has no final component"))?
            .to_string_lossy()
            .into_owned();

        if !members.iter().any(|value| value.as_str() == Some(&member)) {
            members.push(toml::Value::String(member));
            members.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
        }

        let replacement = toml::to_string_pretty(&document)?;

        before_replace();

        #[cfg(unix)]
        {
            let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
            std::io::Write::write_all(&mut temporary, replacement.as_bytes())?;
            temporary.as_file().sync_all()?;

            atomic_exchange(temporary.path(), &manifest)?;
            let displaced = std::fs::read_to_string(temporary.path())?;
            if displaced != source {
                atomic_exchange(temporary.path(), &manifest)?;
                return Ok(false);
            }

            std::fs::File::open(parent)?.sync_all()?;
            Ok(true)
        }

        #[cfg(not(unix))]
        {
            std::io::Seek::rewind(&mut manifest_file)?;
            let mut current = String::new();
            std::io::Read::read_to_string(&mut manifest_file, &mut current)?;
            if current != source || !path_still_names_file(&manifest_file, &manifest)? {
                return Ok(false);
            }

            // Write through the locked handle rather than renaming stale bytes over `manifest`.
            // If a non-cooperating editor atomically replaces the path after the identity check,
            // this handle refers to the unlinked old file and the editor's replacement wins.
            manifest_file.set_len(0)?;
            std::io::Seek::rewind(&mut manifest_file)?;
            std::io::Write::write_all(&mut manifest_file, replacement.as_bytes())?;
            manifest_file.sync_all()?;

            Ok(true)
        }
    })();
    drop(manifest_file);
    drop(lock);

    match result {
        Ok(true) => Ok(()),
        Ok(false) => Err(InitError::WorkspaceManifestConflict {
            project: project.to_path_buf(),
            manifest,
        }),
        Err(source) => Err(InitError::Workspace {
            project: project.to_path_buf(),
            manifest,
            source,
        }),
    }
}

#[cfg(unix)]
fn atomic_exchange(left: &Path, right: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;

    let left = CString::new(left.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let right = CString::new(right.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains NUL"))?;

    #[cfg(any(target_os = "linux", target_os = "android"))]
    // SAFETY: both pointers reference valid NUL-terminated path bytes for the duration of the
    // syscall. `RENAME_EXCHANGE` atomically swaps the two directory entries.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            left.as_ptr(),
            libc::AT_FDCWD,
            right.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    // SAFETY: both pointers reference valid NUL-terminated path bytes for the duration of the
    // call. `RENAME_SWAP` atomically swaps the two directory entries.
    let result = unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            left.as_ptr(),
            libc::AT_FDCWD,
            right.as_ptr(),
            libc::RENAME_SWAP,
        )
    };

    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    )))]
    compile_error!("atomic workspace manifest exchange is unsupported on this Unix target");

    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn path_still_names_file(file: &std::fs::File, path: &Path) -> std::io::Result<bool> {
    #[cfg(windows)]
    {
        return windows_file_identity(file)
            .and_then(|opened| {
                OpenOptions::new()
                    .read(true)
                    .open(path)
                    .map(|current| (opened, current))
            })
            .and_then(|(opened, current)| {
                windows_file_identity(&current).map(|identity| opened == identity)
            });
    }

    #[cfg(not(windows))]
    Ok(true)
}

#[cfg(windows)]
fn windows_file_identity(file: &std::fs::File) -> std::io::Result<(u32, u64)> {
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    let mut information = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: `file` owns a valid Windows handle and the API initializes `information` when
    // it returns nonzero. The borrowed handle remains valid for the duration of the call.
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, information.as_mut_ptr()) };
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: a nonzero return guarantees that the structure was initialized.
    let information = unsafe { information.assume_init() };
    let index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);

    Ok((information.dwVolumeSerialNumber, index))
}

#[cfg(test)]
mod tests;
