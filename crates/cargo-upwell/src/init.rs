//! Catalog-backed project generation through cargo-generate.

mod catalog;
mod publish;
mod workspace;

use std::path::{Path, PathBuf};

use cargo_generate::{GenerateArgs, TemplatePath, Vcs};
pub use catalog::{
    Catalog, CatalogError, GitReference, TemplateEntry, TemplateSource, ToolEntry,
    default_catalog_path,
};
use tempfile::TempDir;
use thiserror::Error;

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
    /// The platform or workspace filesystem cannot perform the required safe publication.
    #[error(
        "workspace manifest `{manifest}` cannot be published safely on this platform or filesystem: {reason}"
    )]
    UnsupportedWorkspacePublication {
        /// Generated project directory that was being registered.
        project: PathBuf,
        /// Parent workspace manifest that was not changed.
        manifest: PathBuf,
        /// Platform or filesystem limitation.
        reason: String,
    },
    /// Publication may have changed the workspace and could not be verified safely.
    #[error(
        "workspace publication for `{manifest}` is indeterminate; preserve `{project}` and recover using `{snapshot}` and `{displaced}`"
    )]
    WorkspacePublicationIndeterminate {
        /// Generated project directory that must be preserved.
        project: PathBuf,
        /// Parent workspace manifest whose state is indeterminate.
        manifest: PathBuf,
        /// Durable immutable pre-publication byte snapshot.
        snapshot: PathBuf,
        /// Durable path holding the inode displaced by publication.
        displaced: PathBuf,
        /// Publication or verification failure.
        #[source]
        source: anyhow::Error,
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

impl InitError {
    fn preserves_generated_project(&self) -> bool {
        matches!(self, Self::WorkspacePublicationIndeterminate { .. })
    }
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
        if !error.preserves_generated_project() {
            let _ = std::fs::remove_dir_all(&path);
        }

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
    workspace::register(project)
}

#[cfg(test)]
fn add_to_parent_workspace_with(
    project: &Path,
    before_replace: impl FnOnce(),
    after_validation: impl FnOnce(),
) -> Result<(), InitError> {
    workspace::register_with(project, before_replace, after_validation)
}

#[cfg(test)]
mod tests;
