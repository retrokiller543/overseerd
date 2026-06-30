//! Service/RPC route resolution and validation.
//!
//! Services live in the [`RpcPlugin`](crate::RpcPlugin) accumulator (not the agnostic
//! `AppRegistry`), so this operates on a `&[ServiceDescriptor]` rather than a registry.

use std::collections::HashSet;

use crate::descriptors::{RpcDescriptor, ServiceDescriptor};
use crate::error::Error;

/// A service header with its RPCs, drawn from the service's own per-service slice.
pub struct ResolvedService {
    pub descriptor: ServiceDescriptor,
    pub rpcs: Vec<&'static RpcDescriptor>,
}

/// Assembles each service header with the RPCs it owns. Services are deduped by type.
pub fn resolved_services(services: &[ServiceDescriptor]) -> Vec<ResolvedService> {
    let mut seen = HashSet::new();

    services
        .iter()
        .filter(|descriptor| seen.insert((descriptor.ty.type_id)()))
        .map(|descriptor| {
            let rpcs = (descriptor.rpcs)()
                .iter()
                .flat_map(|group| group.rpcs.iter())
                .collect();

            ResolvedService {
                descriptor: *descriptor,
                rpcs,
            }
        })
        .collect()
}

/// Validates the RPC surface: unique service ids, non-empty services, and unique RPC
/// names and paths.
pub fn validate_services(services: &[ResolvedService]) -> crate::Result<()> {
    let mut seen_ids = HashSet::new();
    let mut seen_paths: HashSet<String> = HashSet::new();

    for service in services {
        let s = service.descriptor;

        if !seen_ids.insert(s.id) {
            return Err(Error::DuplicateServiceId(s.id.to_string()));
        }

        if service.rpcs.is_empty() {
            return Err(Error::EmptyService(s.name.to_string()));
        }

        let mut seen_rpc_names = HashSet::new();
        for rpc in &service.rpcs {
            if !seen_rpc_names.insert(rpc.name) {
                return Err(Error::DuplicateRpcName {
                    service: s.name.to_string(),
                    rpc: rpc.name.to_string(),
                });
            }

            let path = format!("{}.{}", s.name, rpc.name);
            if !seen_paths.insert(path.clone()) {
                return Err(Error::DuplicateRpcPath(path));
            }
        }
    }

    Ok(())
}
