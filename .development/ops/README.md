# Dispatch operations

The Python tools use this directory's locked uv project. Runtime dependencies are empty;
the development group contains Ruff and basedpyright. Run Python through
`uv run --locked --project .development/ops python`.

`common.sh` stops launchers while either the coordinator's `.git/betterlite/maintenance`
or `logs/maintenance.STOP` exists. The dispatcher checks maintenance again before
each invocation and while waiting for a vendor seat. Removing these files is an
explicit operator action.

Task checkouts use private Git directories with hardlinked immutable objects. The
`betterlite.coordinator` Git setting points at the shared journal and lock directory.
`isolate.py` refuses migration while a process uses the checkout. Existing linked
metadata is archived under `betterlite/isolation/<task>`; commits, the staged index,
working files and untracked files survive migration. Retained task branches are reused.
Retire legacy paused processes before restarting their dispatchers with this code;
continuing an old process would continue its old dispatcher and sandbox settings.

Dispatched workers run focused checks and report preflight. Their stop hook checks
the report and prerequisites. The dispatcher runs the complete gate before either
review pass. Standalone workers retain the full stop-hook gate. `GATE_BUILD_SLOTS`
bounds concurrent cargo check commands separately from model seats; `next-slot.sh`
sets four slots with three build jobs each. Gate history lives under
`betterlite/<task>/gate/runs/`; each invocation also records elapsed time and exit code.

`postgres_bridge.py` checks the loopback fixture before launching its command and
exposes only that fixed endpoint through a private Unix socket. It supplies the
connection URL and sandbox socket permission. The listener exists only for the
command's lifetime. A failed transport is an infrastructure failure, never a passing
database check.

Quota markers wait for the announced reset, with a thirty-minute fallback. Network
failures wait one minute. Authentication failures remain blocked until the operator
repairs authentication and removes the corresponding `betterlite/limits/<vendor>`
file. An expired marker allows one shared probe. Partial worker sessions resume by
explicit session ID and only on the same runner and model.

The warm-up fingerprint includes helper code, cases, the pinned oracle environment
and cache-key implementation. A successful warm-up persists the fingerprint; a
failed one remains retryable. Blocked tasks keep their build caches. Compiler-cache
growth at the next server start preserves 64 GiB free and caps growth at 24 GiB.
The PR watcher requires the `gate.yml` workflow and a commit containing the merge.
After that verification, a clean idle checkout is archived under
`refs/betterlite/completed/<task>` and removed. A late edit or live process preserves
the checkout. Blocked or paused checkouts are never cleaned this way.

Check the operations code:

```sh
uv run --locked --project .development/ops --group dev ruff check .development/ops
uv run --locked --project .development/ops --group dev basedpyright --project .development/ops/pyrightconfig.json
for script in .development/ops/*.sh; do bash -n "$script"; done
```

The existing lint job runs these checks when operations code changes.

`usage.py <journal-root>` reports measured invocation time and available usage records.
Claude's cumulative cost and output-token totals are counted once per session;
Codex's per-turn usage is added. Missing usage is reported as incomplete. The dollar
figure is an API-equivalent estimate from CLI records, not a subscription bill.
