use std::sync::Arc;

use overseerd_app::{
    App, ContributionId, Plugin, PluginContributionKind, PluginContributions, PluginId,
};
use overseerd_di::{Component, ComponentDescriptor};

/// A component contributed entirely through public direct-crate plugin APIs.
struct ThirdPartyComponent;

impl Component for ThirdPartyComponent {
    const ID: &'static str = "third_party_plugin_component";
    const NAME: &'static str = "ThirdPartyPluginComponent";

    type Handle = Arc<Self>;

    fn into_handle(self) -> Self::Handle {
        Arc::new(self)
    }
}

/// A third-party plugin authored without framework-private APIs.
#[derive(Default)]
struct ThirdPartyPlugin;

impl Plugin for ThirdPartyPlugin {
    const ID: PluginId = overseerd_app::namespaced_id!(PluginId, "third-party/direct-app-plugin");

    fn contribute(self, contributions: &mut PluginContributions) {
        contributions.component(
            overseerd_app::namespaced_id!(ContributionId, "third-party/component"),
            ComponentDescriptor::of::<ThirdPartyComponent>(),
        );
    }
}

#[test]
fn direct_crate_exports_support_third_party_plugins() {
    let prepared = App::<()>::builder("direct-third-party-plugin")
        .register_plugin::<ThirdPartyPlugin>()
        .prepare()
        .expect("third-party plugin prepares");
    let plugin = prepared
        .plugin_plan()
        .resolution()
        .plugin(ThirdPartyPlugin::ID)
        .expect("third-party plugin is effective");
    let contribution = prepared.plugin_plan().emitted_contributions()[0];

    assert_eq!(plugin.id(), ThirdPartyPlugin::ID);
    assert_eq!(contribution.kind(), PluginContributionKind::Component);
    assert_eq!(
        contribution.provenance().contributor(),
        overseerd_app::Contributor::Plugin(ThirdPartyPlugin::ID)
    );
    assert!(
        prepared
            .registry()
            .components
            .iter()
            .any(|component| component.id == ThirdPartyComponent::ID)
    );
}
