critical_filter := '''
  binary_id(=overseerd)
  | binary_id(=cargo-overseerd)
  | binary_id(=cargo-overseerd::command)
  | binary_id(=cargo-overseerd::probe)
  | binary_id(=cargo-overseerd::bin/cargo-overseerd)
  | binary_id(=overseerd-app)
  | binary_id(=overseerd-app::third_party_plugin)
  | binary_id(=overseerd-app::third_party_protocol)
  | binary_id(=overseerd-axum)
  | binary_id(=overseerd-axum-json-ws)
  | binary_id(=overseerd-axum-macros)
  | binary_id(=overseerd-axum-stomp)
  | binary_id(=overseerd-client)
  | binary_id(=overseerd-config)
  | binary_id(=overseerd-core)
  | binary_id(=overseerd-di)
  | binary_id(=overseerd-dirs)
  | binary_id(=overseerd-hooks)
  | binary_id(=overseerd-jobs)
  | binary_id(=overseerd-macros-core)
  | binary_id(=overseerd-rpc)
  | binary_id(=overseerd-transport)
  | binary_id(=overseerd-tooling-schema)
  | binary_id(=overseerd-test-utils::process)
  | binary_id(=overseerd-test-utils::temp)
  | binary_id(=overseerd::app_commands)
  | binary_id(=overseerd::app_definition)
  | binary_id(=overseerd::axum_middleware)
  | binary_id(=overseerd::builtins)
  | binary_id(=overseerd::by_value)
  | binary_id(=overseerd::client)
  | binary_id(=overseerd::config_macro)
  | binary_id(=overseerd::config_reload)
  | binary_id(=overseerd::config_reload_hardening)
  | binary_id(=overseerd::dep_swap)
  | binary_id(=overseerd::dependency_injection)
  | binary_id(=overseerd::factory)
  | binary_id(=overseerd::hook_abort)
  | binary_id(=overseerd::hook_custom)
  | binary_id(=overseerd::hook_lifecycle)
  | binary_id(=overseerd::hooks)
  | binary_id(=overseerd::middleware)
  | binary_id(=overseerd::plugin_composition)
  | binary_id(=overseerd::provider_primitives)
  | binary_id(=overseerd::providers)
  | binary_id(=overseerd::rpc_hardening)
  | binary_id(=overseerd::scope_local_providers)
  | binary_id(=overseerd::scopes)
  | binary_id(=overseerd::status_codes)
  | binary_id(=overseerd::streaming)
  | binary_id(=overseerd::third_party_protocol)
  | binary_id(=overseerd::tooling_probe_process)
  | binary_id(=overseerd::type_registration)
  | binary_id(=overseerd-config::namespace_and_defaults)
  | binary_id(=overseerd-config::substitution)
  | binary_id(=overseerd-core::resolver_set_performance)
  | binary_id(=overseerd-di::memory_contracts)
  | binary_id(=overseerd-example-http)
  | binary_id(=overseerd-example-http::client)
  | binary_id(=overseerd-example-http::extractors)
  | binary_id(=overseerd-example-http::openapi)
  | binary_id(=overseerd-example-http::routes)
  | binary_id(=overseerd-example-http::stomp)
  | binary_id(=overseerd-example-http::topic_generics)
  | binary_id(=overseerd-example-http::ws)
  | binary_id(=overseerd-example-daemon::bin/overseerd-example-daemon)
'''

test:
    just test-critical
    just test-extended
    just test-doc

test-critical:
    #!/usr/bin/env bash
    set -euo pipefail

    nextest_version="$(cargo nextest --version)"
    [[ "$nextest_version" == "cargo-nextest 0.9.140 "* ]]
    cargo nextest run \
        --workspace \
        --all-features \
        --locked \
        --profile ci-critical \
        --filterset '{{ critical_filter }}'

test-extended:
    #!/usr/bin/env bash
    set -euo pipefail

    nextest_version="$(cargo nextest --version)"
    [[ "$nextest_version" == "cargo-nextest 0.9.140 "* ]]
    cargo nextest run \
        --workspace \
        --all-features \
        --locked \
        --profile ci-extended \
        --filterset 'all() - ({{ critical_filter }})'

test-doc:
    cargo test --doc --workspace --all-features --locked

test-config:
    #!/usr/bin/env bash
    set -euo pipefail

    nextest_version="$(cargo nextest --version)"
    [[ "$nextest_version" == "cargo-nextest 0.9.140 "* ]]
    cargo nextest show-config version --profile ci-critical
    cargo nextest show-config version --profile ci-extended
    cargo nextest show-config test-groups \
        --workspace \
        --all-features \
        --locked \
        --profile ci-critical >/dev/null
    cargo nextest show-config test-groups \
        --workspace \
        --all-features \
        --locked \
        --profile ci-extended >/dev/null

test-archive archive:
    #!/usr/bin/env bash
    set -euo pipefail

    nextest_version="$(cargo nextest --version)"
    [[ "$nextest_version" == "cargo-nextest 0.9.140 "* ]]
    cargo nextest archive \
        --workspace \
        --all-features \
        --locked \
        --archive-file '{{ archive }}'

test-critical-archive archive:
    just _ci-test-critical '{{ archive }}' "$PWD"

test-extended-archive archive:
    just _ci-test-extended '{{ archive }}' "$PWD"

_ci-test-critical archive workspace:
    #!/usr/bin/env bash
    set -euo pipefail

    nextest_version="$(cargo nextest --version)"
    [[ "$nextest_version" == "cargo-nextest 0.9.140 "* ]]
    cargo nextest run \
        --archive-file '{{ archive }}' \
        --workspace-remap '{{ workspace }}' \
        --profile ci-critical \
        --filterset '{{ critical_filter }}'

_ci-test-extended archive workspace:
    #!/usr/bin/env bash
    set -euo pipefail

    nextest_version="$(cargo nextest --version)"
    [[ "$nextest_version" == "cargo-nextest 0.9.140 "* ]]
    cargo nextest run \
        --archive-file '{{ archive }}' \
        --workspace-remap '{{ workspace }}' \
        --profile ci-extended \
        --filterset 'all() - ({{ critical_filter }})'

bench *targets:
    #!/usr/bin/env bash
    set -euo pipefail

    targets=({{ targets }})

    if [[ ${#targets[@]} -eq 0 ]]; then
        cargo bench --manifest-path benchmarks/Cargo.toml --locked
    else
        args=()

        for target in "${targets[@]}"; do
            args+=(--bench "$target")
        done

        cargo bench --manifest-path benchmarks/Cargo.toml --locked "${args[@]}"
    fi
