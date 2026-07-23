use overseerd::{AppBuilder, AppRegistry, AppRuntime, Plugin, Protocol, ProtocolPlugin, app};

/// Test protocol accumulated by the named application host.
#[derive(Default)]
struct TestPlugin;

impl Plugin for TestPlugin {
    fn register(&self, _registry: &mut AppRegistry) {}
}

/// Built protocol used only to type-check host expansion.
struct TestProtocol;

impl Protocol for TestProtocol {
    type Error = overseerd_app::Error;
}

impl ProtocolPlugin for TestPlugin {
    type Protocol = TestProtocol;
    type Error = overseerd_app::Error;

    const SCOPES: &'static [&'static dyn overseerd::Scope] = &[];

    fn build(self, _runtime: &AppRuntime) -> Result<Self::Protocol, Self::Error> {
        Ok(TestProtocol)
    }
}

app! {
    pub app TestApplication {
        name: "named-app-test",
        protocol: TestPlugin,
    }
}

app! {
    app DirectoryConfigApplication {
        name: "named-directory-config-test",
        protocol: TestPlugin,
        managers: {
            directories: { root: std::env::temp_dir() },
            config: {},
        },
    }
}

fn assert_builder(_builder: AppBuilder<TestPlugin>) {}

#[test]
fn named_app_creates_independent_typed_builders() {
    assert_builder(TestApplication::builder().expect("first builder"));
    assert_builder(TestApplication::builder().expect("second builder"));
}

#[test]
fn named_app_loads_directory_backed_config_fallibly() {
    assert_builder(DirectoryConfigApplication::builder().expect("directory config loads"));
}
