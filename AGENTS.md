# betterlite agent map

betterlite is a Rust workspace. The binary crate lives in `crates/betterlite`;
later gateway, configuration, provider, and routing crates live beside it.

Run `cargo gate run B000` for the complete task gate. During development,
`cargo fmt --all`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
and `cargo nextest run --workspace --locked --no-tests=warn` cover formatting,
linting, and tests. Use `cargo deny --locked check` for dependency policy.

Task definitions and the gate live under `.development/`. Read
`.development/roles/worker.md` for worker workflow and
`.development/rules.yaml` for coding and repository rules before changing code.
