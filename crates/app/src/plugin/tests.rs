use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use overseerd_config::{ConfigManager, ConfigProperties, Toml};
use overseerd_core::TypeDescriptor;
use overseerd_di::{
    BoxedComponent, Component, ComponentConstructionContext, ComponentDescriptor,
    ComponentFactoryDescriptor, ProviderDescriptor, Singleton,
};

use super::{Plugin, PluginCatalog, PluginContributions, ProtocolPluginRegistrar};
use crate::composition::{PluginRelation, RelationKind, RelationTarget};
use crate::{
    App, AppBuilder, AppHost, AppRegistry, ExecutionMode, ProtocolDefinition, ScopeTopology,
};

static PROTOCOL_DISCOVERIES: AtomicUsize = AtomicUsize::new(0);
static PROTOCOL_CONTRIBUTIONS: AtomicUsize = AtomicUsize::new(0);
static APPLICATION_DISCOVERIES: AtomicUsize = AtomicUsize::new(0);
static APPLICATION_CONTRIBUTIONS: AtomicUsize = AtomicUsize::new(0);
static STATEFUL_VALUES: AtomicUsize = AtomicUsize::new(0);
static DEFAULT_DISCOVERIES: AtomicUsize = AtomicUsize::new(0);
static DEFAULT_CONTRIBUTIONS: AtomicUsize = AtomicUsize::new(0);
static REPLACEMENT_DISCOVERIES: AtomicUsize = AtomicUsize::new(0);
static REPLACEMENT_CONTRIBUTIONS: AtomicUsize = AtomicUsize::new(0);

const TEST_SLOT: crate::PluginSlotId =
    crate::namespaced_id!(crate::PluginSlotId, "test/default-slot");

struct ProtocolComponent;

impl Component for ProtocolComponent {
    const ID: &'static str = "protocol_plugin_component";
    const NAME: &'static str = "ProtocolPluginComponent";

    type Handle = Arc<Self>;

    fn into_handle(self) -> Self::Handle {
        Arc::new(self)
    }
}

struct ApplicationComponent;

impl Component for ApplicationComponent {
    const ID: &'static str = "application_plugin_component";
    const NAME: &'static str = "ApplicationPluginComponent";

    type Handle = Arc<Self>;

    fn into_handle(self) -> Self::Handle {
        Arc::new(self)
    }
}

#[derive(serde::Deserialize)]
struct PluginConfig {
    pub _enabled: bool,
}

impl ConfigProperties for PluginConfig {
    const NAME: &'static str = "PluginConfig";
}

fn erase_unreachable(_component: &BoxedComponent) -> BoxedComponent {
    unreachable!("plugin lowering does not erase providers")
}

static PLUGIN_PROVIDER: ProviderDescriptor = ProviderDescriptor {
    trait_ty: TypeDescriptor::of::<ProtocolComponent>("PluginTrait"),
    concrete_ty: TypeDescriptor::of::<ApplicationComponent>(ApplicationComponent::NAME),
    qualifier: "application_plugin_component",
    primary: true,
    priority: 0,
    ordering: &[],
    erase: erase_unreachable,
};

fn construct_protocol_component(
    _context: &mut ComponentConstructionContext,
) -> Pin<Box<dyn Future<Output = overseerd_di::Result<BoxedComponent>> + Send + '_>> {
    unreachable!("plugin preparation must not construct components")
}

fn construct_application_component(
    _context: &mut ComponentConstructionContext,
) -> Pin<Box<dyn Future<Output = overseerd_di::Result<BoxedComponent>> + Send + '_>> {
    unreachable!("plugin preparation must not construct components")
}

fn no_dependencies() -> Vec<overseerd_core::DependencyDescriptor> {
    Vec::new()
}

static PROTOCOL_FACTORIES: [ComponentFactoryDescriptor; 1] = [ComponentFactoryDescriptor {
    construct: construct_protocol_component,
    dependencies: no_dependencies,
    default: true,
}];

static APPLICATION_FACTORIES: [ComponentFactoryDescriptor; 1] = [ComponentFactoryDescriptor {
    construct: construct_application_component,
    dependencies: no_dependencies,
    default: true,
}];

fn protocol_factories() -> &'static [ComponentFactoryDescriptor] {
    &PROTOCOL_FACTORIES
}

fn application_factories() -> &'static [ComponentFactoryDescriptor] {
    &APPLICATION_FACTORIES
}

static PROTOCOL_COMPONENT: ComponentDescriptor = ComponentDescriptor {
    id: ProtocolComponent::ID,
    name: ProtocolComponent::NAME,
    ty: TypeDescriptor::of::<ProtocolComponent>(ProtocolComponent::NAME),
    scope: &Singleton,
    factories: protocol_factories,
    hooks: overseerd_hooks::no_hooks,
};

static APPLICATION_COMPONENT: ComponentDescriptor = ComponentDescriptor {
    id: ApplicationComponent::ID,
    name: ApplicationComponent::NAME,
    ty: TypeDescriptor::of::<ApplicationComponent>(ApplicationComponent::NAME),
    scope: &Singleton,
    factories: application_factories,
    hooks: overseerd_hooks::no_hooks,
};

#[derive(Default)]
struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    const ID: crate::PluginId = crate::namespaced_id!(crate::PluginId, "test/protocol-plugin");

    fn auto_discover(&mut self) {
        PROTOCOL_DISCOVERIES.fetch_add(1, Ordering::SeqCst);
    }

    fn contribute(self, contributions: &mut PluginContributions) {
        PROTOCOL_CONTRIBUTIONS.fetch_add(1, Ordering::SeqCst);
        contributions.component(
            crate::namespaced_id!(crate::ContributionId, "test/protocol-component"),
            PROTOCOL_COMPONENT,
        );
    }
}

#[derive(Default)]
struct ApplicationPlugin;

impl Plugin for ApplicationPlugin {
    const ID: crate::PluginId = crate::namespaced_id!(crate::PluginId, "test/application-plugin");
    const RELATIONS: &'static [PluginRelation] = &[PluginRelation::new(
        RelationKind::Requires,
        RelationTarget::Plugin(ProtocolPlugin::ID),
    )];

    fn auto_discover(&mut self) {
        APPLICATION_DISCOVERIES.fetch_add(1, Ordering::SeqCst);
    }

    fn contribute(self, contributions: &mut PluginContributions) {
        APPLICATION_CONTRIBUTIONS.fetch_add(1, Ordering::SeqCst);
        contributions.component(
            crate::namespaced_id!(crate::ContributionId, "test/application-component"),
            APPLICATION_COMPONENT,
        );
    }
}

#[derive(Default)]
struct DuplicateContributionPlugin;

impl Plugin for DuplicateContributionPlugin {
    const ID: crate::PluginId =
        crate::namespaced_id!(crate::PluginId, "test/duplicate-contribution");

    fn contribute(self, contributions: &mut PluginContributions) {
        const ID: crate::ContributionId =
            crate::namespaced_id!(crate::ContributionId, "test/duplicate");

        contributions.component(ID, PROTOCOL_COMPONENT);
        contributions.component(ID, APPLICATION_COMPONENT);
    }
}

struct StatefulPlugin {
    value: usize,
}

#[derive(Default)]
struct AllContributionKindsPlugin;

impl Plugin for AllContributionKindsPlugin {
    const ID: crate::PluginId = crate::namespaced_id!(crate::PluginId, "test/all-kinds");

    fn contribute(self, contributions: &mut PluginContributions) {
        contributions.component(
            crate::namespaced_id!(crate::ContributionId, "test/all-kinds-component"),
            APPLICATION_COMPONENT,
        );
        contributions.provider(
            crate::namespaced_id!(crate::ContributionId, "test/all-kinds-provider"),
            PLUGIN_PROVIDER,
        );
        contributions.config::<PluginConfig>(
            crate::namespaced_id!(crate::ContributionId, "test/all-kinds-config"),
            "plugin",
        );
    }
}

#[derive(Default)]
struct DefaultPlugin;

impl Plugin for DefaultPlugin {
    const ID: crate::PluginId = crate::namespaced_id!(crate::PluginId, "test/default-plugin");

    fn auto_discover(&mut self) {
        DEFAULT_DISCOVERIES.fetch_add(1, Ordering::SeqCst);
    }

    fn contribute(self, _contributions: &mut PluginContributions) {
        DEFAULT_CONTRIBUTIONS.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct ReplacementPlugin;

impl Plugin for ReplacementPlugin {
    const ID: crate::PluginId = crate::namespaced_id!(crate::PluginId, "test/replacement-plugin");

    fn auto_discover(&mut self) {
        REPLACEMENT_DISCOVERIES.fetch_add(1, Ordering::SeqCst);
    }

    fn contribute(self, _contributions: &mut PluginContributions) {
        REPLACEMENT_CONTRIBUTIONS.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct DefaultProtocol;

impl ProtocolDefinition for DefaultProtocol {
    type Prepared = ();
    type Error = crate::Error;

    const ID: crate::ProtocolId = crate::namespaced_id!(crate::ProtocolId, "test/default-protocol");
    const SCOPE_TOPOLOGY: ScopeTopology = ScopeTopology::empty();

    fn register_plugins(plugins: &mut ProtocolPluginRegistrar) {
        plugins.optional_default::<DefaultPlugin>(TEST_SLOT);
    }

    fn register(&self, _registry: &mut AppRegistry) {}

    fn prepare(
        self,
        _context: &crate::ValidationContext<'_>,
    ) -> Result<Self::Prepared, Self::Error> {
        Ok(())
    }
}

struct ReplacementHost;

impl AppHost for ReplacementHost {
    type Protocol = DefaultProtocol;

    fn declare_plugins(plugins: &mut super::ApplicationPluginRegistrar) {
        plugins.replace::<ReplacementPlugin>(TEST_SLOT);
    }

    fn builder() -> Result<AppBuilder<Self::Protocol>, overseerd_config::ConfigError> {
        Ok(
            App::<DefaultProtocol>::builder("static-plugin-declarations")
                .config_source(ConfigManager::<Toml>::empty())
                .auto_discover(),
        )
    }
}

impl Default for StatefulPlugin {
    fn default() -> Self {
        Self { value: 41 }
    }
}

impl Plugin for StatefulPlugin {
    const ID: crate::PluginId = crate::namespaced_id!(crate::PluginId, "test/stateful");

    fn auto_discover(&mut self) {
        self.value += 1;
    }

    fn contribute(self, _contributions: &mut PluginContributions) {
        STATEFUL_VALUES.fetch_add(self.value, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct TestProtocol;

impl ProtocolDefinition for TestProtocol {
    type Prepared = ();
    type Error = crate::Error;

    const ID: crate::ProtocolId = crate::namespaced_id!(crate::ProtocolId, "test/plugin-plan");
    const SCOPE_TOPOLOGY: ScopeTopology = ScopeTopology::empty();

    fn register_plugins(plugins: &mut ProtocolPluginRegistrar) {
        plugins.mandatory::<ProtocolPlugin>();
    }

    fn register(&self, _registry: &mut AppRegistry) {}

    fn prepare(
        self,
        context: &crate::ValidationContext<'_>,
    ) -> Result<Self::Prepared, Self::Error> {
        assert_eq!(context.plugin_plan().resolution().plugins().len(), 2);

        Ok(())
    }
}

#[test]
fn protocol_and_application_plugins_lower_once_in_resolved_order() {
    PROTOCOL_DISCOVERIES.store(0, Ordering::SeqCst);
    PROTOCOL_CONTRIBUTIONS.store(0, Ordering::SeqCst);
    APPLICATION_DISCOVERIES.store(0, Ordering::SeqCst);
    APPLICATION_CONTRIBUTIONS.store(0, Ordering::SeqCst);

    let prepared = App::<TestProtocol>::builder("plugin-plan")
        .auto_discover()
        .register_plugin::<ApplicationPlugin>()
        .prepare()
        .expect("plugin plan prepares");
    let plugins = prepared.plugin_plan().resolution().plugins();
    let components = &prepared.registry().components;

    assert_eq!(plugins[0].id(), ProtocolPlugin::ID);
    assert_eq!(plugins[1].id(), ApplicationPlugin::ID);
    let protocol_index = components
        .iter()
        .position(|component| component.id == ProtocolComponent::ID)
        .expect("protocol plugin component is lowered");
    let application_index = components
        .iter()
        .position(|component| component.id == ApplicationComponent::ID)
        .expect("application plugin component is lowered");

    assert!(protocol_index < application_index);
    assert_eq!(PROTOCOL_DISCOVERIES.load(Ordering::SeqCst), 1);
    assert_eq!(PROTOCOL_CONTRIBUTIONS.load(Ordering::SeqCst), 1);
    assert_eq!(APPLICATION_DISCOVERIES.load(Ordering::SeqCst), 1);
    assert_eq!(APPLICATION_CONTRIBUTIONS.load(Ordering::SeqCst), 1);
}

#[test]
fn duplicate_contribution_ids_are_typed_errors() {
    let result = App::<()>::builder("duplicate-contribution")
        .register_plugin::<DuplicateContributionPlugin>()
        .prepare();

    let Err(error) = result else {
        panic!("duplicate contribution must fail");
    };

    assert!(matches!(
        error,
        crate::Error::PluginPlan(super::PluginPlanError::DuplicateContribution { .. })
    ));
}

#[test]
fn independently_prepared_apps_own_distinct_retained_plugin_state() {
    STATEFUL_VALUES.store(0, Ordering::SeqCst);

    App::<()>::builder("first-stateful-app")
        .auto_discover()
        .register_plugin::<StatefulPlugin>()
        .prepare()
        .expect("first stateful plugin prepares");
    App::<()>::builder("second-stateful-app")
        .auto_discover()
        .register_plugin::<StatefulPlugin>()
        .prepare()
        .expect("second stateful plugin prepares");

    assert_eq!(STATEFUL_VALUES.load(Ordering::SeqCst), 84);
}

#[test]
fn discovery_is_skipped_when_the_builder_does_not_enable_it() {
    STATEFUL_VALUES.store(0, Ordering::SeqCst);

    App::<()>::builder("undiscovered-stateful-app")
        .register_plugin::<StatefulPlugin>()
        .prepare()
        .expect("stateful plugin prepares without discovery");

    assert_eq!(STATEFUL_VALUES.load(Ordering::SeqCst), 41);
}

#[test]
fn every_app_neutral_contribution_kind_lowers_with_metadata() {
    let mut catalog = PluginCatalog::new();
    let mut registry = AppRegistry::default();

    catalog.register::<AllContributionKindsPlugin>();

    let plan = catalog
        .freeze(
            ProtocolPluginRegistrar::new(crate::namespaced_id!(
                crate::ProtocolId,
                "test/all-kinds"
            )),
            false,
        )
        .expect("all contribution kinds freeze")
        .lower(&mut registry);
    let kinds: Vec<_> = plan
        .emitted_contributions()
        .iter()
        .map(|contribution| contribution.kind())
        .collect();

    assert_eq!(registry.components.len(), 1);
    assert_eq!(registry.providers.len(), 1);
    assert_eq!(registry.config_bindings.len(), 1);
    assert_eq!(
        kinds,
        [
            super::PluginContributionKind::Component,
            super::PluginContributionKind::Provider,
            super::PluginContributionKind::ConfigBinding,
        ]
    );
}

#[test]
fn every_app_neutral_contribution_kind_participates_in_app_preparation() {
    let prepared = App::<()>::builder("all-plugin-contribution-kinds")
        .config_source(
            ConfigManager::<Toml>::from_str(
                r#"
                    [plugin]
                    _enabled = true
                "#,
            )
            .expect("plugin config parses"),
        )
        .register_plugin::<AllContributionKindsPlugin>()
        .prepare()
        .expect("all plugin contribution kinds validate");

    assert_eq!(prepared.registry().providers.len(), 1);
    assert_eq!(prepared.registry().config_bindings.len(), 1);
    assert_eq!(prepared.plugin_plan().emitted_contributions().len(), 3);
    assert_eq!(*prepared.protocol(), ());
}

#[tokio::test]
async fn built_apps_retain_the_prepared_plugin_record() {
    let prepared = App::<()>::builder("built-plugin-record")
        .register_plugin::<StatefulPlugin>()
        .prepare()
        .expect("plugin record prepares");
    let expected = prepared.plugin_plan().clone();
    let app = prepared.build().await.expect("application builds");

    assert_eq!(app.plugin_plan(), &expected);
}

#[test]
fn replacement_executes_only_the_selected_plugin() {
    reset_default_counters();

    let prepared = App::<DefaultProtocol>::builder("replace-default")
        .auto_discover()
        .with_plugin_declarations(|plugins| plugins.replace::<ReplacementPlugin>(TEST_SLOT))
        .prepare()
        .expect("protocol default is replaced");

    assert!(
        prepared
            .plugin_plan()
            .resolution()
            .plugin(DefaultPlugin::ID)
            .is_none()
    );
    assert!(
        prepared
            .plugin_plan()
            .resolution()
            .plugin(ReplacementPlugin::ID)
            .is_some()
    );
    assert_eq!(DEFAULT_DISCOVERIES.load(Ordering::SeqCst), 0);
    assert_eq!(DEFAULT_CONTRIBUTIONS.load(Ordering::SeqCst), 0);
    assert_eq!(REPLACEMENT_DISCOVERIES.load(Ordering::SeqCst), 1);
    assert_eq!(REPLACEMENT_CONTRIBUTIONS.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn host_static_declarations_are_applied_before_late_configuration() {
    reset_default_counters();

    let (_context, prepared) = crate::prepare_host::<ReplacementHost>(ExecutionMode::Tooling)
        .await
        .expect("static host plugin declarations prepare");

    assert!(
        prepared
            .plugin_plan()
            .resolution()
            .plugin(DefaultPlugin::ID)
            .is_none()
    );
    assert!(
        prepared
            .plugin_plan()
            .resolution()
            .plugin(ReplacementPlugin::ID)
            .is_some()
    );
}

#[test]
fn suppression_never_executes_the_optional_default() {
    reset_default_counters();

    let prepared = App::<DefaultProtocol>::builder("suppress-default")
        .auto_discover()
        .with_plugin_declarations(|plugins| plugins.suppress(TEST_SLOT))
        .prepare()
        .expect("optional protocol default is suppressed");

    assert!(prepared.plugin_plan().resolution().plugins().is_empty());
    assert_eq!(DEFAULT_DISCOVERIES.load(Ordering::SeqCst), 0);
    assert_eq!(DEFAULT_CONTRIBUTIONS.load(Ordering::SeqCst), 0);
}

#[test]
fn late_registration_does_not_implicitly_replace_an_early_default() {
    let prepared = App::<DefaultProtocol>::builder("late-default-installation")
        .register_plugin::<ReplacementPlugin>()
        .prepare()
        .expect("independent late plugin remains monotonic");

    assert!(
        prepared
            .plugin_plan()
            .resolution()
            .plugin(DefaultPlugin::ID)
            .is_some()
    );
    assert!(
        prepared
            .plugin_plan()
            .resolution()
            .plugin(ReplacementPlugin::ID)
            .is_some()
    );
    assert_eq!(
        prepared
            .plugin_plan()
            .resolution()
            .slot(TEST_SLOT)
            .expect("protocol default retains its slot")
            .id(),
        DefaultPlugin::ID
    );
}

fn reset_default_counters() {
    DEFAULT_DISCOVERIES.store(0, Ordering::SeqCst);
    DEFAULT_CONTRIBUTIONS.store(0, Ordering::SeqCst);
    REPLACEMENT_DISCOVERIES.store(0, Ordering::SeqCst);
    REPLACEMENT_CONTRIBUTIONS.store(0, Ordering::SeqCst);
}

#[test]
fn duplicate_plugin_installations_preserve_both_origins() {
    let result = App::<()>::builder("duplicate-plugin")
        .register_plugin::<StatefulPlugin>()
        .register_plugin::<StatefulPlugin>()
        .prepare();
    let Err(crate::Error::Composition(diagnostics)) = result else {
        panic!("duplicate plugin installation must be a composition error");
    };
    let [crate::CompositionDiagnostic::DuplicatePlugin { first, second, .. }] =
        diagnostics.as_slice()
    else {
        panic!("duplicate plugin report must contain both installations");
    };

    assert_eq!(first.ordinal(), 0);
    assert_eq!(second.ordinal(), 1);
}
