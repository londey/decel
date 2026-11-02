# decel task runner

default: check

fmt:
    cargo fmt --all

check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo nextest run --workspace
    cargo deny check

publish-dry-run:
    cargo publish --workspace --dry-run

run *ARGS:
    cargo run -p decel -- {{ARGS}}
