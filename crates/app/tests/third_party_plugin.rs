use std::sync::Arc;

#[cfg(feature = "cli")]
use upwell_app::PluginCliRegistrar;
use upwell_app::{
    App, ContributionId, Plugin, PluginContributionKind, PluginContributions, PluginId,
};
use upwell_core::Descriptor;
use upwell_di::{Component, ComponentDescriptor};

/// A component contributed entirely through public direct-crate plugin APIs.
struct ThirdPartyComponent;

impl Component for ThirdPartyComponent {
    type Handle = Arc<Self>;

    const ID: &'static str = "third_party_plugin_component";
    const NAME: &'static str = "ThirdPartyPluginComponent";

    fn into_handle(self) -> Self::Handle {
        Arc::new(self)
    }
}

impl Descriptor<ComponentDescriptor> for ThirdPartyComponent {
    const DESCRIPTOR: ComponentDescriptor = ComponentDescriptor::of::<Self>();
}

/// A third-party plugin authored without framework-private APIs.
#[derive(Default)]
struct ThirdPartyPlugin;

/// Direct-crate global arguments contributed through public plugin APIs.
#[derive(clap::Args)]
#[cfg(feature = "cli")]
struct ThirdPartyArgs {
    /// Enables direct-crate plugin output.
    #[arg(long)]
    enabled: bool,
}

impl Plugin for ThirdPartyPlugin {
    const ID: PluginId = upwell_app::namespaced_id!(PluginId, "third-party/direct-app-plugin");

    fn contribute(self, contributions: &mut PluginContributions) {
        contributions.component::<ThirdPartyComponent>(upwell_app::namespaced_id!(
            ContributionId,
            "third-party/component"
        ));
    }

    #[cfg(feature = "cli")]
    fn cli(&self, cli: &mut PluginCliRegistrar) {
        cli.args::<ThirdPartyArgs>(upwell_app::namespaced_id!(
            ContributionId,
            "third-party/args"
        ));
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
    let contribution = &prepared.plugin_plan().emitted_contributions()[0];

    assert_eq!(plugin.id(), ThirdPartyPlugin::ID);
    assert_eq!(contribution.kind(), PluginContributionKind::Component);
    assert_eq!(
        contribution.provenance().contributor(),
        upwell_app::Contributor::Plugin(ThirdPartyPlugin::ID)
    );
    assert!(
        prepared
            .registry()
            .components
            .iter()
            .any(|component| component.id == ThirdPartyComponent::ID)
    );
    assert_eq!(prepared.plugin_plan().resolution().plugins().len(), 1);
}
