#[cfg(feature = "tooling")]
#[test]
fn prepared_axum_projects_only_retained_controller_and_middleware_facts() {
    let document = overseerd_app::App::<super::Axum>::builder("axum-tooling")
        .config_source(overseerd_config::ConfigManager::<overseerd_config::Dynamic>::empty())
        .prepare()
        .expect("Axum prepares")
        .tooling_document()
        .expect("Axum tooling projects");
    let protocol = document
        .resources
        .iter()
        .find(|resource| resource.id == "protocol:overseerd/axum")
        .expect("Axum protocol resource exists");
    let summary = &protocol.facets["protocol:overseerd/axum/tooling/summary"].value;

    assert_eq!(summary["controller_count"], 0);
    assert_eq!(summary["middleware_count"], 0);
    assert!(
        document
            .relationships
            .iter()
            .filter(|relationship| {
                relationship.from == "protocol:overseerd/axum"
                    && relationship
                        .to
                        .starts_with("protocol:overseerd/axum/tooling/")
            })
            .all(|relationship| {
                relationship.kind == overseerd_app::tooling_schema::RelationshipKind::Contains
            })
    );
}

#[cfg(all(feature = "tooling", feature = "ws"))]
#[test]
fn prepared_axum_projects_websocket_path_and_protocol_identity() {
    use std::sync::Arc;

    use axum::extract::ws::WebSocket;
    use overseerd_app::AppRuntime;
    use overseerd_di::ScopeContainer;

    /// WebSocket protocol used only to retain endpoint metadata.
    struct ToolingWsProtocol;

    impl crate::WebsocketProtocol for ToolingWsProtocol {
        type Payload = ();
        type Outcome = ();
        type Options = ();
        type BuildError = std::convert::Infallible;

        fn build(
            _controllers: &[crate::WsControllerDescriptor],
            _runtime: &AppRuntime,
            _options: Self::Options,
        ) -> Result<Self, Self::BuildError> {
            Ok(Self)
        }

        async fn serve(
            self: Arc<Self>,
            _socket: WebSocket,
            _connection: Arc<ScopeContainer>,
            _shutdown: crate::WsShutdown,
        ) {
        }
    }

    use super::AxumAppBuilder as _;

    let document = overseerd_app::App::<super::Axum>::builder("axum-websocket-tooling")
        .config_source(overseerd_config::ConfigManager::<overseerd_config::Dynamic>::empty())
        .register_ws::<ToolingWsProtocol>("/events")
        .prepare()
        .expect("Axum websocket prepares")
        .tooling_document()
        .expect("Axum websocket tooling projects");
    let endpoint = document
        .resources
        .iter()
        .find(|resource| resource.labels.get("kind") == Some(&String::from("websocket-endpoint")))
        .expect("websocket endpoint resource exists");

    assert_eq!(endpoint.labels["path"], "/events");
    assert!(
        endpoint.labels["protocol-type"].ends_with("ToolingWsProtocol"),
        "prepared endpoint retains stable protocol type identity"
    );
    assert!(document.relationships.iter().any(|relationship| {
        relationship.kind == overseerd_app::tooling_schema::RelationshipKind::Contains
            && relationship.from == "protocol:overseerd/axum"
            && relationship.to == endpoint.id
    }));
}
