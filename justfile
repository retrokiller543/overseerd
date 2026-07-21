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
