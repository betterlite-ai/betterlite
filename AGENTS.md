# betterlite agent map

betterlite is a Rust workspace. The binary crate lives in `crates/betterlite`;
later gateway, configuration, provider, and routing crates live beside it.

Run `cargo gate run <ID>` for the complete task gate. It runs only the tests the
task's own commands name; a push to main runs the whole suite. During development,
`cargo fmt --all` and `cargo clippy -p <crate> --lib --locked -- -D warnings`
on the crates you changed cover formatting and linting; use `--tests` instead of
`--lib` when you changed a tests.rs too. The gate's clippy check lints all targets,
so run `cargo gate run <ID> --only clippy` once before you finish. CI lints every package
the change can affect, so don't use `--workspace`. `cargo gate run <ID> --only <check>`
runs one of the task's test commands. Use `cargo deny --locked check` for dependency policy.

In a dispatched run (`GATE_DISPATCHED=1`) nothing builds or checks on this machine:
the cargo on the PATH refuses build, check, clippy, test and nextest. Format the Rust
files you changed with `rustfmt --edition 2024` and commit; CI runs every check on the
pull request, and a red run comes back with its errors.

Run the oracle with `cargo run --locked -p betterlite-oracle -- --self-check tools/oracle/cases`.
`--self-check` runs LiteLLM on both sides against the same mock provider, so it checks
the cases only. Omit it to compare a built betterlite gateway or the native helpers
with LiteLLM; a task check on helper cases never uses it.

Helper cases cache LiteLLM's answers in two folders. Runs read
`~/.cache/betterlite-oracle` first, then the worktree's `target/oracle-answers`,
and store a new answer in the worktree one. Only a clean main checkout runs with
`BETTERLITE_ORACLE_CACHE=shared` to fill the shared folder; leave it unset in a
task worktree. `--self-check` never uses the cache.

Task definitions and the gate live under `.development/`. Read
`.development/roles/worker.md` for worker workflow and
`.development/rules.yaml` for coding and repository rules before changing code.
