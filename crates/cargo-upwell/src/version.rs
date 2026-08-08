use std::sync::OnceLock;

use cargo_upwell::TOOLING_SCHEMA_VERSION;

pub(crate) fn version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();

    VERSION.get_or_init(|| {
        let commit = option_env!("CARGO_UPWELL_GIT_COMMIT").unwrap_or("unknown");
        let commit = commit.get(..12).unwrap_or(commit);
        let dirty = match option_env!("CARGO_UPWELL_GIT_DIRTY") {
            Some("true") => " (dirty)",
            _ => "",
        };

        format!(
            "{}\ntooling schema {}\ngit {commit}{dirty}\nbuilt for {} ({}) with {}",
            env!("CARGO_PKG_VERSION"),
            TOOLING_SCHEMA_VERSION,
            option_env!("CARGO_UPWELL_BUILD_TARGET").unwrap_or("unknown"),
            option_env!("CARGO_UPWELL_BUILD_PROFILE").unwrap_or("unknown"),
            option_env!("CARGO_UPWELL_RUSTC_VERSION").unwrap_or("unknown"),
        )
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn rich_version_reports_binary_schema_and_build_identity() {
        let version = super::version();

        assert!(version.starts_with(env!("CARGO_PKG_VERSION")));
        assert!(version.contains("tooling schema "));
        assert!(version.contains("\ngit "));
        assert!(version.contains("\nbuilt for "));
    }
}
