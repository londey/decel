# decel task runner

default: check

fmt:
    cargo fmt --all

check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo nextest run --workspace
    cargo deny check

# Cargo treats registry crates as immutable, so a previous dry-run's build of
# an unpublished 0.1.0 crate can be reused stale; clear those first.
publish-dry-run:
    #!/usr/bin/env bash
    set -euo pipefail
    for f in $(grep -l "registry/src/-" target/*/deps/decel_*.d 2>/dev/null || true); do
        base="${f%.d}"; rm -f "$base".* "$(dirname "$base")/lib$(basename "$base")".*
    done
    cargo publish --workspace --dry-run

run *ARGS:
    cargo run -p decel -- {{ARGS}}
