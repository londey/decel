# decel task runner

default: check

fmt:
    cargo fmt --all

check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo nextest run --workspace
    cargo deny check

# Cargo treats registry crates as immutable: a previous dry-run's unpacked
# sources and builds of the unpublished 0.1.0 crates (in cargo's temporary
# local registry) would be reused stale, so clear them first.
publish-dry-run:
    #!/usr/bin/env bash
    set -euo pipefail
    home="${CARGO_HOME:-$HOME/.cargo}"
    for d in "$home"/registry/src/-* "$home"/registry/cache/-* "$home"/registry/index/-*; do
        [ -e "$d" ] && rm -rf "$d"
    done
    rm -rf target/package
    for f in $(grep -l "registry/src/-" target/*/deps/decel_*.d 2>/dev/null || true); do
        base="${f%.d}"; rm -f "$base".* "$(dirname "$base")/lib$(basename "$base")".*
    done
    cargo publish --workspace --dry-run

run *ARGS:
    cargo run -p decel -- {{ARGS}}
