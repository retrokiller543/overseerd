use std::collections::HashMap;

use crate::{
    descriptors::RpcHandler,
    registry::Registry,
    Error, RpcCallContext, RpcResponse,
};

/// Routes incoming RPC calls to their registered handlers by path.
///
/// Paths take the form `ServiceName.rpc_name`, matching the convention used
/// in registry validation.
pub struct RpcRouter {
    routes: HashMap<String, RpcHandler>,
}

impl RpcRouter {
    pub fn from_registry(registry: &Registry) -> Self {
        let mut routes = HashMap::new();

        for service in &registry.services {
            for rpc in service.rpcs {
                let path = format!("{}.{}", service.name, rpc.name);
                routes.insert(path, rpc.handler);
            }
        }

        Self { routes }
    }

    pub async fn dispatch(&self, path: &str, ctx: RpcCallContext) -> crate::Result<RpcResponse> {
        let handler = self
            .routes
            .get(path)
            .ok_or_else(|| Error::RouteNotFound(path.to_string()))?;

        handler(ctx).await
    }

    /// Returns the number of registered routes.
    pub fn route_count(&self) -> usize {
        self.routes.len()
    }

    /// Returns an iterator over all registered route paths.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.routes.keys().map(String::as_str)
    }
}
