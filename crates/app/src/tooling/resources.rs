use std::collections::BTreeMap;

use overseerd_core::{
    Cardinality, DependencyDescriptor, ResolutionMode, Singleton, StaticScope, Transient,
};
use overseerd_di::ProviderDescriptor;
use overseerd_tooling_schema::{Provenance, RelationshipKind, ResourceKind};

use super::snapshot::{ComponentSnapshot, ConstructionPlanEntry};
use super::{
    Projection, component_id, config_binding_id, contribution_provenance, lifecycle_resource,
    provider_id, scope_id,
};
use crate::ProtocolDefinition;

impl<D: ProtocolDefinition> Projection<'_, D> {
    pub(super) fn project_application(&mut self) {
        self.resource(
            "framework",
            ResourceKind::Contributor,
            "overseerd",
            Some(Provenance {
                owner: Some(String::from("framework")),
                origin: Some(String::from("framework")),
                ..Provenance::default()
            }),
            BTreeMap::new(),
        );
        self.resource(
            "application",
            ResourceKind::Application,
            self.app.name(),
            None,
            BTreeMap::new(),
        );
        let protocol = format!("protocol:{}", self.app.protocol_id().as_str());

        self.resource(
            &protocol,
            ResourceKind::Protocol,
            self.app.protocol_id().as_str(),
            Some(Provenance {
                owner: Some(protocol.clone()),
                origin: Some(String::from("protocol")),
                ..Provenance::default()
            }),
            BTreeMap::from([(
                String::from("protocol-id"),
                self.app.protocol_id().as_str().to_string(),
            )]),
        );
        self.relationship(RelationshipKind::Validates, &protocol, "application", []);

        let host = self.app.host_lifecycle();

        if let Some(host) = host {
            for (name, callback) in [
                ("setup", host.setup),
                ("configure", host.configure),
                ("before_build", host.before_build),
                ("after_build", host.after_build),
                ("serve", host.serve),
            ] {
                self.lifecycle(name, "host-phase", Some(callback), false);
            }
        }

        for name in ["prepare", "build"] {
            self.lifecycle(name, "construction-phase", None, false);
        }

        for (name, repeatable) in [
            ("startup", false),
            ("shutdown", false),
            ("config_reload", true),
        ] {
            self.lifecycle(name, "runtime-event", None, repeatable);
        }

        if host.is_some() {
            self.lifecycle_order("setup", "configure");
            self.lifecycle_order("configure", "before_build");
            self.lifecycle_order("before_build", "prepare");
        }

        self.lifecycle_order("prepare", "build");

        if host.is_some() {
            self.lifecycle_order("build", "after_build");
        }

        if host.is_some() {
            self.lifecycle_order("after_build", "serve");
        }
    }

    pub(super) fn project_scopes(&mut self) {
        self.framework_scope(
            Singleton::ID.as_str(),
            Singleton::NAME,
            Singleton::RANK,
            false,
        );
        self.framework_scope(
            Transient::ID.as_str(),
            Transient::NAME,
            Transient::RANK,
            true,
        );

        for boundary in self.app.scope_topology().boundaries() {
            let id = scope_id(boundary.id().as_str());
            let parent = scope_id(boundary.parent().id().as_str());

            self.resource(
                &id,
                ResourceKind::Scope,
                boundary.name(),
                Some(Provenance {
                    owner: Some(format!("protocol:{}", self.app.protocol_id().as_str())),
                    origin: Some(String::from("protocol")),
                    ..Provenance::default()
                }),
                BTreeMap::from([
                    (String::from("rank"), boundary.rank().to_string()),
                    (String::from("scope-id"), boundary.id().as_str().to_string()),
                ]),
            );
            self.relationship(RelationshipKind::OpensScope, &parent, &id, []);
        }
    }

    pub(super) fn project_components(&mut self) {
        let components = self.app.tooling_snapshot().components().to_vec();

        for component in &components {
            self.project_component(component);
        }

        self.project_construction_plan(
            Singleton::ID.as_str(),
            self.app.tooling_snapshot().root_plan().to_vec(),
        );

        for boundary in self.app.scope_topology().boundaries() {
            let Some(order) = self.app.tooling_snapshot().scope_plan(&boundary.id()) else {
                continue;
            };

            self.project_construction_plan(boundary.id().as_str(), order.to_vec());
        }
    }

    pub(super) fn project_providers(&mut self) {
        let registry = self.app.registry();
        let providers = registry.providers.clone();

        for provider in &providers {
            self.project_provider(provider);
        }

        let mut traits: Vec<_> = providers
            .iter()
            .map(|provider| provider.trait_ty.type_id)
            .collect();

        traits.sort();
        traits.dedup();

        for trait_ty in traits {
            let mut ordered: Vec<_> = providers
                .iter()
                .filter(|provider| provider.trait_ty.type_id == trait_ty)
                .filter_map(|provider| {
                    self.app
                        .provider_order(trait_ty, provider.concrete_ty.type_id)
                        .map(|ordinal| (ordinal, provider))
                })
                .collect();

            ordered.sort_by_key(|(ordinal, _)| *ordinal);

            for pair in ordered.windows(2) {
                let [(_, before), (_, after)] = pair else {
                    continue;
                };
                let trait_resource =
                    self.type_resource((before.trait_ty.type_name)(), before.trait_ty.name);

                self.relationship(
                    RelationshipKind::OrdersBefore,
                    &provider_id(before),
                    &provider_id(after),
                    [
                        (String::from("trait-target"), trait_resource),
                        (
                            String::from("rust-trait"),
                            (before.trait_ty.type_name)().to_string(),
                        ),
                    ],
                );
            }
        }
    }

    pub(super) fn project_config_bindings(&mut self) {
        let bindings = self.app.registry().config_bindings.clone();

        for binding in &bindings {
            let id = config_binding_id((binding.ty.type_name)(), &binding.path);
            let ty = self.type_resource((binding.ty.type_name)(), binding.ty.name);
            let provenance = self
                .app
                .plugin_plan()
                .selected_contribution(&id)
                .map(contribution_provenance);

            self.resource(
                &id,
                ResourceKind::ConfigBinding,
                binding.ty.name,
                provenance,
                BTreeMap::from([
                    (String::from("path"), binding.path.clone()),
                    (String::from("redacted"), String::from("true")),
                    (String::from("value-exported"), String::from("false")),
                ]),
            );
            self.relationship(RelationshipKind::Binds, &id, &ty, []);
        }

        self.project_config_sources();
    }

    pub(super) fn project_hooks(&mut self) {
        let components = self.app.tooling_snapshot().components().to_vec();

        for component in &components {
            let component_resource = component_id(component.descriptor.id);

            for hook in &component.hooks {
                let id = format!(
                    "hook:{}:{}:{}",
                    component.descriptor.id, hook.kind, hook.ordinal
                );

                self.resource(
                    &id,
                    ResourceKind::Hook,
                    hook.kind,
                    None,
                    BTreeMap::from([(String::from("ordinal"), hook.ordinal.to_string())]),
                );
                self.relationship(RelationshipKind::Hooks, &component_resource, &id, []);

                if let Some(lifecycle) = lifecycle_resource(hook.kind) {
                    self.relationship(RelationshipKind::Hooks, &id, lifecycle, []);
                }

                for dependency in &hook.dependencies {
                    self.project_dependency(&id, dependency);
                }
            }
        }
    }

    fn framework_scope(&mut self, id: &str, name: &str, rank: u8, transient: bool) {
        self.resource(
            &scope_id(id),
            ResourceKind::Scope,
            name,
            Some(Provenance {
                owner: Some(String::from("framework")),
                origin: Some(String::from("framework")),
                ..Provenance::default()
            }),
            BTreeMap::from([
                (String::from("rank"), rank.to_string()),
                (String::from("scope-id"), id.to_string()),
                (String::from("transient"), transient.to_string()),
            ]),
        );
    }

    fn lifecycle(&mut self, name: &str, category: &str, callback: Option<bool>, repeatable: bool) {
        let mut labels = BTreeMap::from([
            (String::from("category"), category.to_string()),
            (String::from("repeatable"), repeatable.to_string()),
        ]);

        if let Some(callback) = callback {
            labels.insert(String::from("callback"), callback.to_string());
        }

        self.resource(
            &format!("lifecycle:{name}"),
            ResourceKind::Lifecycle,
            name,
            None,
            labels,
        );
    }

    fn lifecycle_order(&mut self, before: &str, after: &str) {
        self.relationship(
            RelationshipKind::OrdersBefore,
            &format!("lifecycle:{before}"),
            &format!("lifecycle:{after}"),
            [],
        );
    }

    fn project_construction_plan(&mut self, scope: &str, order: Vec<ConstructionPlanEntry>) {
        let mut previous: Option<String> = None;

        for (ordinal, entry) in order.into_iter().enumerate() {
            let id = component_id(entry.descriptor.id);
            let resource = self
                .document
                .resources
                .iter_mut()
                .find(|resource| resource.id == id)
                .expect("retained construction component is projected");

            resource
                .labels
                .insert(String::from("construction-plan"), scope.to_string());
            resource
                .labels
                .insert(String::from("plan-ordinal"), ordinal.to_string());
            resource.labels.insert(
                String::from("plan-entry-kind"),
                if entry.has_factory {
                    String::from("factory-construction")
                } else {
                    String::from("seeded-verification")
                },
            );

            if let Some(previous) = previous {
                self.relationship(
                    RelationshipKind::OrdersBefore,
                    &previous,
                    &id,
                    [(String::from("scope"), scope.to_string())],
                );
            }

            previous = Some(id);
        }
    }

    fn project_component(&mut self, component: &ComponentSnapshot) {
        let descriptor = component.descriptor;
        let id = component_id(descriptor.id);
        let ty = self.type_resource((descriptor.ty.type_name)(), descriptor.ty.name);
        let scope = scope_id(descriptor.scope.id().as_str());
        let construction = if descriptor.scope.id() == Transient::ID {
            if component.has_factory {
                "transient-on-demand"
            } else {
                "manual-transient"
            }
        } else if component.seed_destination.is_some() {
            "scope-seed"
        } else if component.has_factory {
            "planned-factory"
        } else if component.seeded {
            "seeded"
        } else {
            "manual-unseeded"
        };
        let labels = BTreeMap::from([
            (String::from("component-id"), descriptor.id.to_string()),
            (
                String::from("rust-type"),
                (descriptor.ty.type_name)().to_string(),
            ),
            (
                String::from("scope-id"),
                descriptor.scope.id().as_str().to_string(),
            ),
            (String::from("construction"), construction.to_string()),
        ]);

        self.resource(&id, ResourceKind::Component, descriptor.name, None, labels);
        self.relationship(RelationshipKind::Provides, &id, &ty, []);
        self.relationship(
            RelationshipKind::DependsOn,
            &id,
            &scope,
            [(String::from("role"), String::from("scope"))],
        );

        for dependency in &component.dependencies {
            self.project_dependency(&id, dependency);
        }
    }

    fn project_dependency(&mut self, owner: &str, dependency: &DependencyDescriptor) {
        let target = self.type_resource((dependency.ty.type_name)(), dependency.ty.name);
        let mut labels = dependency_labels(dependency);

        if dependency.config {
            let bindings: Vec<_> = self
                .app
                .registry()
                .config_bindings
                .iter()
                .filter(|binding| binding.ty.type_id == dependency.ty.type_id)
                .filter(|binding| dependency.qualifier.is_none_or(|path| binding.path == path))
                .collect();

            if let [binding] = bindings.as_slice() {
                self.relationship_map(
                    RelationshipKind::DependsOn,
                    owner,
                    &config_binding_id((binding.ty.type_name)(), &binding.path),
                    labels,
                );

                return;
            }

            labels.insert(
                String::from("binding-resolution"),
                String::from("ambiguous"),
            );
            labels.insert(
                String::from("binding-cardinality"),
                bindings.len().to_string(),
            );
        }

        self.relationship_map(RelationshipKind::DependsOn, owner, &target, labels);
    }

    fn project_provider(&mut self, provider: &ProviderDescriptor) {
        let id = provider_id(provider);
        let trait_resource =
            self.type_resource((provider.trait_ty.type_name)(), provider.trait_ty.name);
        let component = self.app.component_resource_id(provider.concrete_ty.type_id);
        let order = self
            .app
            .provider_order(provider.trait_ty.type_id, provider.concrete_ty.type_id);
        let mut labels = BTreeMap::from([
            (String::from("primary"), provider.primary.to_string()),
            (String::from("priority"), provider.priority.to_string()),
            (String::from("qualifier"), provider.qualifier.to_string()),
            (
                String::from("rust-trait"),
                (provider.trait_ty.type_name)().to_string(),
            ),
            (String::from("trait-target"), trait_resource.clone()),
        ]);

        if let Some(order) = order {
            labels.insert(String::from("processing-ordinal"), order.to_string());
        }

        self.resource(
            &id,
            ResourceKind::Provider,
            provider.trait_ty.name,
            None,
            labels,
        );
        self.relationship(RelationshipKind::Provides, &id, &trait_resource, []);

        if let Some(component) = &component {
            self.relationship(RelationshipKind::Provides, component, &id, []);
        }
    }

    fn project_config_sources(&mut self) {
        let mut previous: Option<String> = None;

        for (ordinal, path) in self.app.config_sources().iter().enumerate() {
            let display = path.display().to_string();

            if display.is_empty() || display == "<in-memory>" {
                continue;
            }

            let id = format!("config-source:{ordinal}");

            self.resource(
                &id,
                ResourceKind::Contribution,
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("config source"),
                Some(Provenance {
                    owner: Some(String::from("framework")),
                    origin: Some(String::from("config-source")),
                    ordinal: Some(ordinal as u32),
                    ..Provenance::default()
                }),
                BTreeMap::from([
                    (String::from("path"), display),
                    (String::from("merge-ordinal"), ordinal.to_string()),
                    (String::from("values-exported"), String::from("false")),
                ]),
            );

            if let Some(previous) = previous {
                self.relationship(
                    RelationshipKind::OrdersBefore,
                    &previous,
                    &id,
                    [(String::from("order"), String::from("config-merge"))],
                );
            }

            previous = Some(id);
        }
    }
}

fn dependency_labels(dependency: &DependencyDescriptor) -> BTreeMap<String, String> {
    let cardinality = match dependency.cardinality {
        Cardinality::One => "one",
        Cardinality::Collection => "collection",
        Cardinality::Keyed => "keyed",
    };
    let resolution = match dependency.resolution {
        ResolutionMode::Eager => "eager",
        ResolutionMode::Deferred => "deferred",
        ResolutionMode::Lazy => "lazy",
        ResolutionMode::Fresh => "fresh",
    };
    let mut labels = BTreeMap::from([
        (String::from("cardinality"), cardinality.to_string()),
        (String::from("dynamic"), dependency.dynamic.to_string()),
        (String::from("name"), dependency.name.to_string()),
        (String::from("optional"), dependency.optional.to_string()),
        (String::from("resolution"), resolution.to_string()),
    ]);

    if let Some(qualifier) = dependency.qualifier {
        labels.insert(String::from("qualifier"), qualifier.to_string());
    }

    labels
}
