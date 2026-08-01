use std::path::Path;

/// Returns whether a path ends with the platform-independent component suffix.
pub fn path_ends_with_components(path: impl AsRef<Path>, suffix: &[&str]) -> bool {
    let components = path
        .as_ref()
        .components()
        .map(|component| component.as_os_str())
        .collect::<Vec<_>>();

    components.len() >= suffix.len()
        && components[components.len() - suffix.len()..]
            .iter()
            .zip(suffix)
            .all(|(actual, expected)| actual == expected)
}
