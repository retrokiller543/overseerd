use std::sync::Arc;

use upwell::{
    App, Component, ComponentDescriptor, CompositionDirective, Descriptor, InstallationOrigin,
    InstallationProvenance, Plugin, PluginContributionKind, PluginContributions, PluginDeclaration,
    PluginId, ProtocolId, resolve_early_plugins,
};

/// A component contributed through the facade-only plugin contract.
struct FacadePluginComponent;

impl Component for FacadePluginComponent {
    type Handle = Arc<Self>;

    const ID: &'static str = "facade_plugin_component";
    const NAME: &'static str = "FacadePluginComponent";

    fn into_handle(self) -> Self::Handle {
        Arc::new(self)
    }
}

impl Descriptor<ComponentDescriptor> for FacadePluginComponent {
    const DESCRIPTOR: ComponentDescriptor = ComponentDescriptor::of::<Self>();
}

/// A third-party plugin using only facade exports.
#[derive(Default)]
struct FacadePlugin;

impl Plugin for FacadePlugin {
    const ID: PluginId = upwell::namespaced_id!(PluginId, "third-party/facade-plugin");

    fn contribute(self, contributions: &mut PluginContributions) {
        upwell::contribute! {
            to contributions,
            components: [
                "third-party/facade-component" => type FacadePluginComponent,
            ],
        }

        #[cfg(feature = "tooling")]
        {
            contributions.tooling().resource("worker", "Facade worker");
            contributions.tooling().relationship(
                upwell::tooling::ToolingRelationshipKind::Contains,
                upwell::tooling::ToolingEndpoint::Owner,
                upwell::tooling::ToolingEndpoint::Resource("worker"),
            );
        }
    }
}

#[cfg(feature = "tooling")]
#[test]
fn third_party_facade_plugin_projects_owner_scoped_generic_metadata() {
    let document = App::<()>::builder("facade-plugin-tooling")
        .register_plugin::<FacadePlugin>()
        .prepare()
        .expect("facade-only plugin prepares")
        .tooling_document()
        .expect("facade-only plugin tooling projects");

    assert!(
        document
            .resources
            .iter()
            .any(|resource| resource.id == "plugin:third-party/facade-plugin/tooling/worker")
    );
    assert!(document.relationships.iter().any(|relationship| {
        relationship.kind == upwell::tooling::RelationshipKind::Contains
            && relationship.from == "plugin:third-party/facade-plugin"
            && relationship.to == "plugin:third-party/facade-plugin/tooling/worker"
    }));
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
