//! Tests for the `@` directory namespace and macro-supplied field defaults
//! (`DefaultSpec`). Like `substitution.rs`, everything runs against a `MapResolver`
//! so the cases are deterministic and env-free.

// Target structs deserialize on error paths and never read some fields.
#![allow(dead_code)]

use std::collections::HashMap;

use overseerd_config::{
    ConfigError, ConfigErrorKind, ConfigStr, ConfigValue, DefaultSpec, EnumTag, MapResolver,
    ResolverChain, from_value,
};
use serde::Deserialize;

/// A string leaf parsed through the real placeholder grammar.
fn s(raw: &str) -> ConfigValue {
    ConfigValue::Str(ConfigStr::parse(raw).unwrap())
}

/// A table node from `(key, value)` pairs.
fn table(entries: Vec<(&str, ConfigValue)>) -> ConfigValue {
    let mapped = entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();

    ConfigValue::Table(mapped)
}

/// A resolver chain backed by an in-memory map (the env / namespace stand-in).
fn resolvers(pairs: &[(&str, &str)]) -> ResolverChain {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();

    ResolverChain(vec![Box::new(MapResolver(map))])
}

/// The failure kind behind a `ConfigError`, regardless of path context.
fn kind(error: &ConfigError) -> &ConfigErrorKind {
    match error {
        ConfigError::At { kind, .. } => kind,
        ConfigError::Bare(kind) => kind,
    }
}

#[test]
fn namespace_key_resolves_from_the_resolver_chain() {
    #[derive(Debug, Deserialize)]
    struct Cfg {
        socket: String,
    }

    let tree = table(vec![("socket", s("${@runtime}/app.sock"))]);
    let chain = resolvers(&[("@runtime", "/run/app")]);

    let cfg: Cfg = from_value(&tree, &chain).unwrap();

    assert_eq!(cfg.socket, "/run/app/app.sock");
}

#[test]
fn namespace_key_uses_inline_default_when_unresolved() {
    #[derive(Debug, Deserialize)]
    struct Cfg {
        dir: String,
    }

    let tree = table(vec![("dir", s("${@cache:/tmp}/blobs"))]);
    let chain = resolvers(&[]);

    let cfg: Cfg = from_value(&tree, &chain).unwrap();

    assert_eq!(cfg.dir, "/tmp/blobs");
}

#[test]
fn unknown_namespace_key_errors() {
    #[derive(Debug, Deserialize)]
    struct Cfg {
        dir: String,
    }

    let tree = table(vec![("dir", s("${@nope}"))]);
    let chain = resolvers(&[]);

    let error = from_value::<Cfg>(&tree, &chain).unwrap_err();

    assert!(matches!(
        kind(&error),
        ConfigErrorKind::UnknownNamespaceKey { key } if key == "@nope"
    ));
}

#[test]
fn namespace_key_never_consults_the_config_tree() {
    // A config table literally named `@runtime` must NOT satisfy the namespace key;
    // namespace lookup goes only to the resolver chain, which has no answer here.
    #[derive(Debug, Deserialize)]
    struct Cfg {
        dir: String,
    }

    let bogus = table(vec![("@runtime", s("from_tree"))]);
    let tree = table(vec![("dir", s("${@runtime}")), ("nested", bogus)]);
    let chain = resolvers(&[]);

    let error = from_value::<Cfg>(&tree, &chain).unwrap_err();

    assert!(matches!(
        kind(&error),
        ConfigErrorKind::UnknownNamespaceKey { .. }
    ));
}

#[test]
fn struct_defaults_fill_missing_and_never_override() {
    #[derive(Debug, Deserialize)]
    struct Cfg {
        port: u16,
        host: String,
        addr: String,
    }

    // `port` is present in the file and must win; `host` and the templated `addr` are
    // absent and fall back to their defaults, resolving through the normal pipeline.
    let mut subtree = table(vec![("port", s("8080"))]);
    let defaults = DefaultSpec::Fields(vec![
        ("port".to_string(), "1".to_string()),
        ("host".to_string(), "localhost".to_string()),
        ("addr".to_string(), "${HOST}:9000".to_string()),
    ]);

    defaults.fill_missing(&mut subtree).unwrap();

    let chain = resolvers(&[("HOST", "example.com")]);
    let cfg: Cfg = from_value(&subtree, &chain).unwrap();

    assert_eq!(cfg.port, 8080);
    assert_eq!(cfg.host, "localhost");
    assert_eq!(cfg.addr, "example.com:9000");
}

#[test]
fn fully_defaulted_struct_materializes_from_empty_table() {
    #[derive(Debug, Deserialize)]
    struct Cfg {
        host: String,
    }

    let mut subtree = ConfigValue::Table(Vec::new());
    let defaults = DefaultSpec::Fields(vec![("host".to_string(), "localhost".to_string())]);

    defaults.fill_missing(&mut subtree).unwrap();

    let chain = resolvers(&[]);
    let cfg: Cfg = from_value(&subtree, &chain).unwrap();

    assert_eq!(cfg.host, "localhost");
}

#[test]
fn enum_variant_default_applies_only_to_the_present_variant() {
    #[derive(Debug, Deserialize, PartialEq)]
    enum Storage {
        Memory,
        Disk { path: String },
    }

    // The `Disk` variant is present with its `path` omitted, so the variant default
    // fills it. `Memory`'s (empty) default set is irrelevant.
    let mut subtree = table(vec![("Disk", ConfigValue::Table(Vec::new()))]);
    let defaults = DefaultSpec::Variants {
        tagging: EnumTag::External,
        default: None,
        fields: vec![(
            "Disk".to_string(),
            vec![("path".to_string(), "${DATA}/blobs".to_string())],
        )],
    };

    defaults.fill_missing(&mut subtree).unwrap();

    let chain = resolvers(&[("DATA", "/var/lib/app")]);
    let cfg: Storage = from_value(&subtree, &chain).unwrap();

    assert_eq!(
        cfg,
        Storage::Disk {
            path: "/var/lib/app/blobs".to_string()
        }
    );
}

#[test]
fn enum_unit_variant_is_left_untouched_by_defaults() {
    #[derive(Debug, Deserialize, PartialEq)]
    enum Storage {
        Memory,
        Disk { path: String },
    }

    // A bare-string unit variant carries no fields; fill_missing must be a no-op.
    let mut subtree = s("Memory");
    let defaults = DefaultSpec::Variants {
        tagging: EnumTag::External,
        default: None,
        fields: vec![(
            "Disk".to_string(),
            vec![("path".to_string(), "${DATA}/blobs".to_string())],
        )],
    };

    defaults.fill_missing(&mut subtree).unwrap();

    let chain = resolvers(&[]);
    let cfg: Storage = from_value(&subtree, &chain).unwrap();

    assert_eq!(cfg, Storage::Memory);
}

#[test]
fn default_variant_synthesized_when_none_selected() {
    #[derive(Debug, Deserialize, PartialEq)]
    enum Storage {
        Memory,
        Disk { path: String },
    }

    // No variant named (empty table) and `Memory` is the default unit variant: it is
    // synthesized as a bare tag string.
    let mut subtree = ConfigValue::Table(Vec::new());
    let defaults = DefaultSpec::Variants {
        tagging: EnumTag::External,
        default: Some(("Memory".to_string(), true)),
        fields: vec![],
    };

    defaults.fill_missing(&mut subtree).unwrap();

    let chain = resolvers(&[]);
    let cfg: Storage = from_value(&subtree, &chain).unwrap();

    assert_eq!(cfg, Storage::Memory);
}

#[test]
fn default_struct_variant_synthesized_with_its_field_defaults() {
    #[derive(Debug, Deserialize, PartialEq)]
    enum Storage {
        Memory,
        Disk { path: String },
    }

    // The default is a non-unit variant: it is synthesized as `{ Disk: { ..defaults } }`,
    // and its own field defaults fill the missing `path`.
    let mut subtree = ConfigValue::Table(Vec::new());
    let defaults = DefaultSpec::Variants {
        tagging: EnumTag::External,
        default: Some(("Disk".to_string(), false)),
        fields: vec![(
            "Disk".to_string(),
            vec![("path".to_string(), "${DATA}/blobs".to_string())],
        )],
    };

    defaults.fill_missing(&mut subtree).unwrap();

    let chain = resolvers(&[("DATA", "/var/lib/app")]);
    let cfg: Storage = from_value(&subtree, &chain).unwrap();

    assert_eq!(
        cfg,
        Storage::Disk {
            path: "/var/lib/app/blobs".to_string()
        }
    );
}

#[test]
fn explicit_variant_wins_over_default() {
    #[derive(Debug, Deserialize, PartialEq)]
    enum Storage {
        Memory,
        Disk { path: String },
    }

    // `Disk` is explicitly selected, so the `Memory` default must not be synthesized.
    let mut subtree = table(vec![("Disk", table(vec![("path", s("/explicit"))]))]);
    let defaults = DefaultSpec::Variants {
        tagging: EnumTag::External,
        default: Some(("Memory".to_string(), true)),
        fields: vec![],
    };

    defaults.fill_missing(&mut subtree).unwrap();

    let chain = resolvers(&[]);
    let cfg: Storage = from_value(&subtree, &chain).unwrap();

    assert_eq!(
        cfg,
        Storage::Disk {
            path: "/explicit".to_string()
        }
    );
}

#[test]
fn none_defaults_change_nothing() {
    let mut subtree = table(vec![("port", s("8080"))]);
    let before = subtree.clone();

    DefaultSpec::none().fill_missing(&mut subtree).unwrap();

    assert_eq!(subtree, before);
}
