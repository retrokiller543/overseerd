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

fn assert_builder(_builder: AppBuilder<TestPlugin>) {}

#[test]
fn named_app_creates_independent_typed_builders() {
    assert_builder(TestApplication::builder());
    assert_builder(TestApplication::builder());
}
