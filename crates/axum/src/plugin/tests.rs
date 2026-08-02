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
    assert_eq!(
        protocol
            .display
            .as_ref()
            .and_then(|display| display.label.as_deref()),
        Some("Axum HTTP")
    );
    assert_eq!(
        protocol
            .display
            .as_ref()
            .and_then(|display| display.summary.as_deref()),
        Some("0 controllers, 0 middleware layers")
    );
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

#[cfg(feature = "tooling")]
#[test]
fn prepared_axum_projects_static_http_routes_without_building_runtime() {
    struct ToolingController;

    fn routes() -> Vec<crate::HttpRouteDescriptor> {
        vec![
            crate::HttpRouteDescriptor {
                handler: "health",
                method: "GET",
                path: "/health",
            },
            crate::HttpRouteDescriptor {
                handler: "health",
                method: "GET",
                path: "/ready",
            },
        ]
    }

    let controller = crate::ControllerDescriptor {
        id: "tooling-controller",
        name: "ToolingController",
        ty: overseerd_core::TypeDescriptor::of::<ToolingController>("ToolingController"),
        base: "/api",
        router: |_| panic!("tooling must not build controller router"),
        routes,
    };
    use super::AxumAppBuilder as _;

    let controller = Box::leak(Box::new(controller));

    let document = overseerd_app::App::<super::Axum>::builder("axum-route-tooling")
        .config_source(overseerd_config::ConfigManager::<overseerd_config::Dynamic>::empty())
        .controller_descriptor(controller)
        .prepare()
        .expect("Axum prepares")
        .tooling_document()
        .expect("Axum tooling projects");
    let routes = document
        .resources
        .iter()
        .filter(|resource| resource.labels.get("kind").map(String::as_str) == Some("http-route"))
        .collect::<Vec<_>>();
    let route = routes
        .iter()
        .find(|resource| {
            resource
                .display
                .as_ref()
                .and_then(|display| display.label.as_deref())
                == Some("GET /api/health")
        })
        .expect("health route resource exists");

    assert_eq!(routes.len(), 2);
    assert_ne!(routes[0].id, routes[1].id);
    assert_eq!(
        route
            .display
            .as_ref()
            .and_then(|display| display.label.as_deref()),
        Some("GET /api/health")
    );
    assert_eq!(route.display.as_ref().unwrap().details["handler"], "health");
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
        endpoint
            .display
            .as_ref()
            .and_then(|display| display.label.as_deref())
            .is_some_and(|label| label.ends_with("ToolingWsProtocol"))
    );
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
