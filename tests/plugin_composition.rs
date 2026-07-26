use std::sync::Arc;

use overseerd::{
    App, Component, ComponentDescriptor, CompositionDirective, ContributionId, InstallationOrigin,
    InstallationProvenance, Plugin, PluginContributionKind, PluginContributions, PluginDeclaration,
    PluginId, ProtocolId, resolve_early_plugins,
};

/// A component contributed through the facade-only plugin contract.
struct FacadePluginComponent;

impl Component for FacadePluginComponent {
    const ID: &'static str = "facade_plugin_component";
    const NAME: &'static str = "FacadePluginComponent";

    type Handle = Arc<Self>;

    fn into_handle(self) -> Self::Handle {
        Arc::new(self)
    }
}

/// A third-party plugin using only facade exports.
#[derive(Default)]
struct FacadePlugin;

impl Plugin for FacadePlugin {
    const ID: PluginId = overseerd::namespaced_id!(PluginId, "third-party/facade-plugin");

    fn contribute(self, contributions: &mut PluginContributions) {
        contributions.component(
            overseerd::namespaced_id!(ContributionId, "third-party/facade-component"),
            ComponentDescriptor::of::<FacadePluginComponent>(),
        );
    }
}

#[test]
fn facade_exports_public_composition_contracts() {
    let protocol = ProtocolId::new("test/protocol").expect("valid protocol id");
    let plugin = PluginId::new("third-party/plugin").expect("valid plugin id");
    let provenance = InstallationProvenance::new(InstallationOrigin::ApplicationDeclaration, 0);
    let plan = resolve_early_plugins(
        protocol,
        [CompositionDirective::install(PluginDeclaration::new(
            plugin, provenance,
        ))],
    )
    .expect("public composition plan resolves");

    assert_eq!(plan.protocol(), protocol);
    assert_eq!(plan.plugins()[0].id(), plugin);
}

#[test]
fn facade_exports_public_retained_plugin_contracts() {
    let prepared = App::<()>::builder("facade-plugin")
        .register_plugin::<FacadePlugin>()
        .prepare()
        .expect("facade-only plugin prepares");

    assert!(
        prepared
            .plugin_plan()
            .resolution()
            .plugin(FacadePlugin::ID)
            .is_some()
    );
    assert_eq!(
        prepared.plugin_plan().emitted_contributions()[0].kind(),
        PluginContributionKind::Component
    );
}
