# betterlite agent map

betterlite is a Rust workspace. The binary crate lives in `crates/betterlite`;
later gateway, configuration, provider, and routing crates live beside it.

Run `cargo gate run <ID>` for the complete task gate. During development,
`cargo fmt --all`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
and `cargo nextest run --workspace --locked --no-tests=warn` cover formatting,
linting, and tests. Use `cargo deny --locked check` for dependency policy.

Run the oracle with `cargo run --locked -p betterlite-oracle -- --self-check tools/oracle/cases`.
It compares LiteLLM and betterlite against the same mock provider; omit `--self-check`
when testing a built betterlite gateway.

Task definitions and the gate live under `.development/`. Read
`.development/roles/worker.md` for worker workflow and
`.development/rules.yaml` for coding and repository rules before changing code.
