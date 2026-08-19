use crate::TempFixture;
use upwell_config::{ConfigManager, ResolverChain, Toml};
use upwell_dirs::DirectoriesManager;

/// Owns private application directories and environment-free configuration for a test.
pub struct TestEnvironment {
    root: TempFixture,
}

impl TestEnvironment {
    /// Creates a private test root with a descriptive prefix.
    pub fn new(prefix: &str) -> Self {
        let root = TempFixture::new(prefix);

        Self { root }
    }

    /// Returns a directory manager rooted in this environment.
    pub fn directories(&self) -> DirectoriesManager {
        DirectoriesManager::from_path(self.root.path().to_path_buf())
    }

    /// Returns an empty configuration that never consults process environment variables.
    pub fn config(&self) -> ConfigManager<Toml> {
        ConfigManager::empty().with_resolvers(ResolverChain::empty())
    }
}
