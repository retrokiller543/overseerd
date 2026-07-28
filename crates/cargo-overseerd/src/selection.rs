use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use cargo_metadata::Metadata;
use thiserror::Error;

/// Cargo feature settings used to determine target eligibility and build the selected package.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FeatureSelection {
    /// Whether Cargo default features are disabled.
    pub no_default_features: bool,
    /// Whether every package feature is enabled.
    pub all_features: bool,
    /// Explicit selected package features.
    pub features: Vec<String>,
}

impl FeatureSelection {
    /// Returns the normalized feature arguments supplied to Cargo.
    pub fn normalized_features(&self) -> Vec<String> {
        let mut features = BTreeSet::new();

        for value in &self.features {
            for feature in value
                .split([',', ' '])
                .filter(|feature| !feature.is_empty())
            {
                features.insert(feature.to_string());
            }
        }

        features.into_iter().collect()
    }
}

/// One Cargo binary target that can be shown in selection diagnostics.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct BinaryCandidate {
    /// Cargo target name.
    pub name: String,
    /// Package features Cargo requires before this target can be built.
    pub required_features: Vec<String>,
    /// Required features absent from the current feature selection.
    pub missing_features: Vec<String>,
}

impl BinaryCandidate {
    /// Whether the target is eligible under the current feature selection.
    pub const fn is_eligible(&self) -> bool {
        self.missing_features.is_empty()
    }
}

/// One workspace package that can be shown in selection diagnostics.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PackageCandidate {
    /// Cargo package identifier.
    pub id: String,
    /// Cargo package name.
    pub name: String,
    /// Cargo package version.
    pub version: String,
    /// Absolute package manifest path reported by Cargo.
    pub manifest_path: PathBuf,
    /// Declared ordinary binary targets in stable name order.
    pub binaries: Vec<BinaryCandidate>,
    /// Package `default-run` target, when declared.
    pub default_run: Option<String>,
    /// Whether Cargo considers this package a workspace default member.
    pub default_member: bool,
}

/// One explicitly selected Cargo package and binary target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectedTarget {
    /// Cargo package identifier.
    pub package_id: String,
    /// Cargo package name.
    pub package_name: String,
    /// Cargo package version.
    pub package_version: String,
    /// Absolute package manifest path reported by Cargo.
    pub manifest_path: PathBuf,
    /// Selected Cargo binary target name.
    pub binary_name: String,
    /// Features required by the selected binary target.
    pub required_features: Vec<String>,
}

/// Cargo workspace facts required for deterministic application target selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceCatalog {
    /// Absolute Cargo workspace root.
    pub workspace_root: PathBuf,
    /// Absolute effective Cargo target directory.
    pub target_directory: PathBuf,
    /// Workspace packages that declare at least one ordinary binary target.
    pub packages: Vec<PackageCandidate>,
}

impl WorkspaceCatalog {
    /// Projects Cargo metadata into the stable selection model.
    pub fn from_metadata(metadata: &Metadata, features: &FeatureSelection) -> Self {
        let workspace_members = metadata
            .workspace_members
            .iter()
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>();
        let default_members = metadata
            .workspace_default_members
            .iter()
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>();
        let mut packages = metadata
            .packages
            .iter()
            .filter(|package| workspace_members.contains(&package.id.to_string()))
            .filter_map(|package| {
                let enabled_features = enabled_features(package, features);
                let mut binaries = package
                    .targets
                    .iter()
                    .filter(|target| target.is_bin())
                    .map(|target| {
                        let mut required_features = target.required_features.clone();

                        required_features.sort();
                        required_features.dedup();

                        let missing_features = required_features
                            .iter()
                            .filter(|feature| !enabled_features.contains(feature.as_str()))
                            .cloned()
                            .collect();

                        BinaryCandidate {
                            name: target.name.clone(),
                            required_features,
                            missing_features,
                        }
                    })
                    .collect::<Vec<_>>();

                binaries.sort();

                if binaries.is_empty() {
                    return None;
                }

                Some(PackageCandidate {
                    id: package.id.to_string(),
                    name: package.name.to_string(),
                    version: package.version.to_string(),
                    manifest_path: package.manifest_path.clone().into_std_path_buf(),
                    binaries,
                    default_run: package.default_run.clone(),
                    default_member: default_members.contains(&package.id.to_string()),
                })
            })
            .collect::<Vec<_>>();

        packages.sort_by(|left, right| {
            (&left.name, &left.manifest_path).cmp(&(&right.name, &right.manifest_path))
        });

        Self {
            workspace_root: metadata.workspace_root.clone().into_std_path_buf(),
            target_directory: metadata.target_directory.clone().into_std_path_buf(),
            packages,
        }
    }

    /// Selects one package and binary target using Cargo-native defaults where unambiguous.
    pub fn select(
        &self,
        package: Option<&str>,
        binary: Option<&str>,
    ) -> Result<SelectedTarget, SelectionError> {
        let package = self.select_package(package)?;
        let binary = select_binary(package, binary)?;

        Ok(SelectedTarget {
            package_id: package.id.clone(),
            package_name: package.name.clone(),
            package_version: package.version.clone(),
            manifest_path: package.manifest_path.clone(),
            binary_name: binary.name.clone(),
            required_features: binary.required_features.clone(),
        })
    }

    fn select_package(&self, requested: Option<&str>) -> Result<&PackageCandidate, SelectionError> {
        if let Some(requested) = requested {
            return self
                .packages
                .iter()
                .find(|package| package.name == requested)
                .ok_or_else(|| SelectionError::PackageNotFound {
                    requested: requested.to_string(),
                    candidates: self.packages.clone(),
                });
        }

        let default_members = self
            .packages
            .iter()
            .filter(|package| package.default_member)
            .collect::<Vec<_>>();

        if default_members.len() == 1 {
            return Ok(default_members[0]);
        }

        if default_members.len() > 1 {
            return Err(SelectionError::AmbiguousPackage {
                candidates: default_members.into_iter().cloned().collect(),
            });
        }

        match self.packages.as_slice() {
            [] => Err(SelectionError::NoPackage),
            [package] => Ok(package),
            packages => Err(SelectionError::AmbiguousPackage {
                candidates: packages.to_vec(),
            }),
        }
    }
}

/// A deterministic Cargo package or binary target selection failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum SelectionError {
    /// The workspace has no package declaring an ordinary binary target.
    #[error("the Cargo workspace contains no binary target")]
    NoPackage,
    /// The explicitly requested package is not an eligible workspace package.
    #[error("Cargo package '{requested}' was not found among workspace binary packages")]
    PackageNotFound {
        /// Requested package name.
        requested: String,
        /// Available packages in stable order.
        candidates: Vec<PackageCandidate>,
    },
    /// More than one package remains after applying Cargo workspace defaults.
    #[error("multiple Cargo packages contain binary targets; select one package explicitly")]
    AmbiguousPackage {
        /// Ambiguous packages in stable order.
        candidates: Vec<PackageCandidate>,
    },
    /// The selected package declares no ordinary binary target.
    #[error("Cargo package '{package}' contains no binary target")]
    NoBinary {
        /// Selected package name.
        package: String,
    },
    /// The explicitly requested binary is absent from the selected package.
    #[error("Cargo binary target '{requested}' was not found in package '{package}'")]
    BinaryNotFound {
        /// Selected package name.
        package: String,
        /// Requested binary target name.
        requested: String,
        /// Available binary targets in stable order.
        candidates: Vec<BinaryCandidate>,
    },
    /// A selected binary cannot be built without additional package features.
    #[error("Cargo binary target '{binary}' requires disabled package features")]
    RequiredFeatures {
        /// Selected package name.
        package: String,
        /// Selected binary target name.
        binary: String,
        /// Disabled required features in stable order.
        missing_features: Vec<String>,
    },
    /// More than one eligible binary remains after applying `default-run`.
    #[error("multiple Cargo binary targets are eligible; select one binary explicitly")]
    AmbiguousBinary {
        /// Selected package name.
        package: String,
        /// Ambiguous binary targets in stable order.
        candidates: Vec<BinaryCandidate>,
    },
}

fn select_binary<'a>(
    package: &'a PackageCandidate,
    requested: Option<&str>,
) -> Result<&'a BinaryCandidate, SelectionError> {
    if let Some(requested) = requested {
        let binary = package
            .binaries
            .iter()
            .find(|binary| binary.name == requested)
            .ok_or_else(|| SelectionError::BinaryNotFound {
                package: package.name.clone(),
                requested: requested.to_string(),
                candidates: package.binaries.clone(),
            })?;

        return require_eligible(package, binary);
    }

    if let Some(default_run) = package.default_run.as_deref()
        && let Some(binary) = package
            .binaries
            .iter()
            .find(|binary| binary.name == default_run)
    {
        return require_eligible(package, binary);
    }

    let eligible = package
        .binaries
        .iter()
        .filter(|binary| binary.is_eligible())
        .collect::<Vec<_>>();

    match eligible.as_slice() {
        [] if package.binaries.is_empty() => Err(SelectionError::NoBinary {
            package: package.name.clone(),
        }),
        [] if package.binaries.len() == 1 => require_eligible(package, &package.binaries[0]),
        [] => Err(SelectionError::AmbiguousBinary {
            package: package.name.clone(),
            candidates: package.binaries.clone(),
        }),
        [binary] => Ok(binary),
        binaries => Err(SelectionError::AmbiguousBinary {
            package: package.name.clone(),
            candidates: binaries.iter().copied().cloned().collect(),
        }),
    }
}

fn require_eligible<'a>(
    package: &PackageCandidate,
    binary: &'a BinaryCandidate,
) -> Result<&'a BinaryCandidate, SelectionError> {
    if !binary.is_eligible() {
        return Err(SelectionError::RequiredFeatures {
            package: package.name.clone(),
            binary: binary.name.clone(),
            missing_features: binary.missing_features.clone(),
        });
    }

    Ok(binary)
}

fn enabled_features(
    package: &cargo_metadata::Package,
    selection: &FeatureSelection,
) -> BTreeSet<String> {
    let mut enabled = if selection.all_features {
        package.features.keys().cloned().collect::<BTreeSet<_>>()
    } else {
        selection
            .normalized_features()
            .into_iter()
            .collect::<BTreeSet<_>>()
    };
    let definitions = package
        .features
        .iter()
        .map(|(name, values)| (name.as_str(), values.as_slice()))
        .collect::<BTreeMap<_, _>>();

    if !selection.no_default_features && definitions.contains_key("default") {
        enabled.insert(String::from("default"));
    }

    let mut pending = enabled.iter().cloned().collect::<Vec<_>>();

    while let Some(feature) = pending.pop() {
        let Some(values) = definitions.get(feature.as_str()) else {
            continue;
        };

        for value in *values {
            let referenced = value
                .strip_prefix("dep:")
                .unwrap_or(value)
                .split(['/', '?'])
                .next()
                .unwrap_or(value);

            if definitions.contains_key(referenced) && enabled.insert(referenced.to_string()) {
                pending.push(referenced.to_string());
            }
        }
    }

    enabled
}

#[cfg(test)]
mod tests;
