use overseerd::{
    CompositionDirective, InstallationOrigin, InstallationProvenance, PluginDeclaration, PluginId,
    ProtocolId, resolve_early_plugins,
};

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
