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

Helper cases cache LiteLLM's answers in two folders. Runs read
`~/.cache/betterlite-oracle` first, then the worktree's `target/oracle-answers`,
and store a new answer in the worktree one. Only a clean main checkout runs with
`BETTERLITE_ORACLE_CACHE=shared` to fill the shared folder; leave it unset in a
task worktree. `--self-check` never uses the cache.

Task definitions and the gate live under `.development/`. Read
`.development/roles/worker.md` for worker workflow and
`.development/rules.yaml` for coding and repository rules before changing code.
