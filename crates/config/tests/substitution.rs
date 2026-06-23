//! End-to-end tests for the type-safe placeholder-substitution deserializer. All
//! tests use a `MapResolver` (standing in for the environment) so they are
//! deterministic and env-free.

// Several target structs deserialize on an error path and never read their fields.
#![allow(dead_code)]

use std::collections::HashMap;

use overseer_config::{
    ConfigError, ConfigErrorKind, ConfigStr, ConfigValue, MapResolver, ResolverChain, from_value,
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

/// A resolver chain backed by an in-memory map (the env stand-in).
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
fn full_placeholder_coerces_to_target_scalar_type() {
    #[derive(Debug, Deserialize)]
    struct Cfg {
        flag: bool,
        port: u16,
        ratio: f64,
        name: String,
    }

    let tree = table(vec![
        ("flag", s("${FLAG}")),
        ("port", s("${PORT}")),
        ("ratio", s("${RATIO}")),
        ("name", s("${NAME}")),
    ]);
    let chain = resolvers(&[
        ("FLAG", "true"),
        ("PORT", "8080"),
        ("RATIO", "0.5"),
        ("NAME", "overseer"),
    ]);

    let cfg: Cfg = from_value(&tree, &chain).unwrap();

    assert!(cfg.flag);
    assert_eq!(cfg.port, 8080);
    assert_eq!(cfg.ratio, 0.5);
    assert_eq!(cfg.name, "overseer");
}

#[test]
fn partial_placeholder_renders_into_a_string() {
    #[derive(Debug, Deserialize)]
    struct Cfg {
        url: String,
    }

    let tree = table(vec![("url", s("https://${HOST}/v1"))]);
    let chain = resolvers(&[("HOST", "example.com")]);

    let cfg: Cfg = from_value(&tree, &chain).unwrap();

    assert_eq!(cfg.url, "https://example.com/v1");
}

#[test]
fn partial_placeholder_into_non_string_is_rejected() {
    #[derive(Debug, Deserialize)]
    struct Cfg {
        port: u16,
    }

    let tree = table(vec![("port", s("port-${N}"))]);
    let chain = resolvers(&[("N", "8080")]);

    let error = from_value::<Cfg>(&tree, &chain).unwrap_err();

    assert!(matches!(
        kind(&error),
        ConfigErrorKind::PartialInNonString { target: "u16" }
    ));
}

#[test]
fn inline_default_is_used_when_unresolved() {
    #[derive(Debug, Deserialize)]
    struct Cfg {
        greeting: String,
    }

    let tree = table(vec![("greeting", s("${MISSING:fallback}"))]);
    let chain = resolvers(&[]);

    let cfg: Cfg = from_value(&tree, &chain).unwrap();

    assert_eq!(cfg.greeting, "fallback");
}

#[test]
fn missing_placeholder_without_default_errors() {
    #[derive(Debug, Deserialize)]
    struct Cfg {
        value: String,
    }

    let tree = table(vec![("value", s("${NOPE}"))]);
    let chain = resolvers(&[]);

    let error = from_value::<Cfg>(&tree, &chain).unwrap_err();

    assert!(matches!(
        kind(&error),
        ConfigErrorKind::MissingPlaceholder { key } if key == "NOPE"
    ));
}

#[test]
fn out_of_range_full_placeholder_errors() {
    #[derive(Debug, Deserialize)]
    struct Cfg {
        port: u8,
    }

    let tree = table(vec![("port", s("${PORT}"))]);
    let chain = resolvers(&[("PORT", "8080")]);

    let error = from_value::<Cfg>(&tree, &chain).unwrap_err();

    assert!(matches!(
        kind(&error),
        ConfigErrorKind::OutOfRange { target: "u8", .. }
    ));
}

#[test]
fn uppercase_heuristic_picks_env_for_screaming_keys_and_config_for_lowercase() {
    #[derive(Debug, Deserialize)]
    struct Cfg {
        from_lower: String,
        from_upper: String,
        from_path: String,
    }

    // `greeting` exists both in config and the resolver: lowercase prefers config,
    // SCREAMING prefers the resolver, dotted is always a config path.
    let app = table(vec![("host", s("db.internal"))]);
    let tree = table(vec![
        ("greeting", s("from_config")),
        ("app", app),
        ("from_lower", s("${greeting}")),
        ("from_upper", s("${GREETING}")),
        ("from_path", s("${app.host}")),
    ]);
    let chain = resolvers(&[("greeting", "lower_env"), ("GREETING", "upper_env")]);

    let cfg: Cfg = from_value(&tree, &chain).unwrap();

    assert_eq!(cfg.from_lower, "from_config");
    assert_eq!(cfg.from_upper, "upper_env");
    assert_eq!(cfg.from_path, "db.internal");
}

#[test]
fn transitive_chain_resolves_depth_first() {
    #[derive(Debug, Deserialize)]
    struct Cfg {
        port: u16,
    }

    // port -> raw (config) -> BASE (env). Every hop is a string; only the outermost
    // `u16` drives the typed parse.
    let tree = table(vec![("raw", s("${BASE}")), ("port", s("${raw}"))]);
    let chain = resolvers(&[("BASE", "8080")]);

    let cfg: Cfg = from_value(&tree, &chain).unwrap();

    assert_eq!(cfg.port, 8080);
}

#[test]
fn resolution_cycle_is_detected() {
    #[derive(Debug, Deserialize)]
    struct Cfg {
        value: String,
    }

    let tree = table(vec![
        ("a", s("${b}")),
        ("b", s("${a}")),
        ("value", s("${a}")),
    ]);
    let chain = resolvers(&[]);

    let error = from_value::<Cfg>(&tree, &chain).unwrap_err();

    assert!(matches!(
        kind(&error),
        ConfigErrorKind::ResolutionCycle { .. }
    ));
}

#[test]
fn long_linear_chain_fails_with_depth_error_not_overflow() {
    #[derive(Debug, Deserialize)]
    struct Cfg {
        value: String,
    }

    // A non-repeating chain k0 -> k1 -> ... -> k199. Cycle detection never fires
    // (no key repeats), so without a depth cap this would recurse 200 deep. The cap
    // must turn that into a clean error rather than a stack overflow.
    let mut entries: Vec<(String, ConfigValue)> = (0..200)
        .map(|i| (format!("k{i}"), s(&format!("${{k{}}}", i + 1))))
        .collect();
    entries.push(("k200".to_string(), s("done")));
    entries.push(("value".to_string(), s("${k0}")));

    let owned = table(
        entries
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect(),
    );
    let chain = resolvers(&[]);

    let error = from_value::<Cfg>(&owned, &chain).unwrap_err();

    assert!(matches!(
        kind(&error),
        ConfigErrorKind::ResolutionDepthExceeded { .. }
    ));
}

#[test]
fn absolute_path_resolves_against_full_root_from_subtree() {
    // Deserializing the `app.server` subtree must still resolve the absolute path
    // `${app.server.port}` against the whole tree, not the subtree.
    #[derive(Debug, Deserialize)]
    struct Server {
        addr: String,
    }

    let server = table(vec![
        ("port", s("${PORT:9000}")),
        ("addr", s("127.0.0.1:${app.server.port}")),
    ]);
    let app = table(vec![("server", server)]);
    let root = table(vec![("app", app)]);
    let chain = resolvers(&[]);

    let subtree = root.get_path("app.server").unwrap();
    let cfg: Server = overseer_config::from_value_in(&root, subtree, &chain).unwrap();

    assert_eq!(cfg.addr, "127.0.0.1:9000");
}

#[test]
fn escaped_dollar_is_literal() {
    #[derive(Debug, Deserialize)]
    struct Cfg {
        literal: String,
    }

    let tree = table(vec![("literal", s("$${NOT_A_VAR}"))]);
    let chain = resolvers(&[]);

    let cfg: Cfg = from_value(&tree, &chain).unwrap();

    assert_eq!(cfg.literal, "${NOT_A_VAR}");
}

#[test]
fn nested_structs_and_sequences_resolve() {
    #[derive(Debug, Deserialize)]
    struct Server {
        host: String,
        port: u16,
    }

    #[derive(Debug, Deserialize)]
    struct Cfg {
        server: Server,
        ports: Vec<u16>,
    }

    let server = table(vec![("host", s("${HOST}")), ("port", s("${PORT}"))]);
    let ports = ConfigValue::Array(vec![ConfigValue::Int(1), s("${PORT}")]);
    let tree = table(vec![("server", server), ("ports", ports)]);
    let chain = resolvers(&[("HOST", "localhost"), ("PORT", "9000")]);

    let cfg: Cfg = from_value(&tree, &chain).unwrap();

    assert_eq!(cfg.server.host, "localhost");
    assert_eq!(cfg.server.port, 9000);
    assert_eq!(cfg.ports, vec![1, 9000]);
}
