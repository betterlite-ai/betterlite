# .development

This folder drives the agents that build betterlite, a Rust rewrite of LiteLLM's proxy and Python SDK under the MIT license. The work runs as a pyramid. The architect (the main session, with Max) owns the backlog and merges. A planner turns backlog entries into task files. Workers on cheaper models build one task each in their own worktree, and a reviewer from the other vendor reads every change the gate let through. The gate, a Rust CLI in gate/, decides what's done: every check is a command that exits 0 or doesn't, and every agent's final answer is JSON that must match its schema. LiteLLM 1.105.1 is the behavioral spec, and from B001 on an oracle harness runs it next to betterlite on the same requests.

## The pyramid

| Role | Played by (agents.yaml) | Writes | Answers with |
|---|---|---|---|
| Architect | main Claude session with Max | backlog, specs, merges | numbered questions for Max |
| Planner | Codex gpt-6-astra, xhigh effort | task files on a plan branch | report JSON |
| Worker | Codex gpt-reserve, then gpt-6-astra, then one try on Claude Opus | code on its task branch | report JSON |
| Reviewer | Claude Sonnet (Opus for tasks with review_level security), or Codex gpt-6-astra when the worker was Claude | nothing | review JSON |

Each role has its rules in roles/ and its output format in schema/. The worker and reviewer rows are what `cargo gate dispatch <ID>` runs; the planner is started by the architect as roles/architect.md describes. Models, effort, attempts and timeouts live in agents.yaml, so changing who plays a role is a one-line edit that `cargo gate validate` checks.

The dispatcher climbs the worker ladder. Each attempt runs the worker on the task, reads its report, runs the gate, and checks the report against the repository: every crate added to a Cargo.toml must be a chosen option in the report's prior art. A failure goes back to the next attempt as feedback, and a rung that runs out of attempts, or a worker that answers blocked, hands the task to the next rung. A rung whose vendor is at its usage limit is skipped, and a task gets at most max_worker_attempts worker runs in all. When the gate passes, the reviewer reads the diff against the reviewed rules. Its findings go back to the worker for up to max_review_rounds rounds. A later round gets the earlier findings and the commit that round read: it checks those are fixed and reviews only lines changed since, so a review converges instead of finding new things in settled code. The run ends ready to merge, or with an error the architect handles. Every prompt, answer and log of a run is kept under .git/betterlite/<ID>/.

Inside a run, both CLIs call `cargo gate hook` when the agent tries to stop: Codex through .codex/hooks.json, Claude through the agent files in .claude/agents/. The hook runs the gate and validates the report, and sends the agent back with the failing lines up to max_hook_blocks times. So most failures get fixed inside one attempt, and the dispatcher's own gate run is the backstop.

## Files

- backlog/ holds one file per task, <id>.yaml, with its milestone, status, dependencies and the LiteLLM paths it covers. Each entry also carries ticket fields: parent (the umbrella a part belongs to), split_from (the entry it was split off from), resolution (merged, obsolete, duplicate or superseded, once done), superseded_by (what replaces a duplicate or superseded entry), blocked_reason (one line while blocked) and pr (the pull request that merged it). An entry that ends without code is marked done with its resolution instead of being deleted. The architect owns it; the planner fills in task files and splits oversized entries on a plan branch.
- tasks/B*.yaml holds one task each: description, scope, references, definition of done, checks, oracle cases, deviations, out-of-scope items, pitfalls, review level and budget.
- rules.yaml is the code standard: 35 rules, each with its reason, the checks that enforce it, whether reviewers read for it, and an example of code that breaks it next to the same intent written right.
- checks.yaml pins the CLI tools to exact versions and defines the shared settings, the base checks every task runs, and the optional checks a task can add.
- agents.yaml maps roles to runners, models and limits.
- litellm.yaml pins the LiteLLM release betterlite follows: repository, version, commit, the folders and files that make up LiteLLM outside its enterprise directory, and the paths left out, each with its reason.
- litellm-inventory.txt lists every LiteLLM path under those folders at the pinned commit. `cargo gate inventory <checkout>` writes it from a LiteLLM clone; nobody edits it by hand.
- roles/ holds the rules for the architect, planner, worker and reviewer.
- schema/ holds the JSON Schemas for every format above plus the report and review answers. `cargo gate schema` generates them from the gate's Rust types, and `cargo gate validate` fails when a file breaks its schema or a schema is out of date.
- templates/ holds the files B000 copies to the root byte for byte: workspace manifest with the deny-everything lint table, clippy.toml, deny.toml, typos and ast-grep config, prek hooks, LICENSE and THIRD_PARTY_NOTICES.md.
- gate/ is the gate's source. `cargo gate` runs it from the repository root through the alias in .cargo/config.toml.
- site/index.html is the progress page at https://betterlite-ai.github.io/betterlite/. After every push to main, .github/workflows/pages.yml publishes it next to the backlog, the deviation logs and the merge history, and the page reads them in the browser.

Every path in the inventory has to be accounted for: a backlog entry's covers globs claim it, or litellm.yaml excludes it with a reason. `cargo gate validate` fails while any path is unclaimed or any glob claims nothing, so nothing LiteLLM does outside its enterprise directory can fall out of the backlog. Moving to a newer LiteLLM release means updating litellm.yaml, regenerating the inventory and claiming the new paths.

Every deliberate difference from LiteLLM is written down twice: in the worker's report and in docs/deviations/<task id>.yaml, one file per task, in the format schema/deviations.schema.json describes. The dispatcher refuses a report whose deviations don't match that file, and `cargo gate validate` checks every log. So the LiteLLM bugs betterlite chose not to copy, and why, live in the repository.

## Repository

The code lives at https://github.com/betterlite-ai/betterlite, a public repository in an org whose only member is Max. Every change reaches main through a pull request and the merge queue, with rebase merges only and branches deleted after merge. Every pull request gets auto-merge when it opens, so it merges by itself once the gate check passes. A workflow can't do that step: when GITHUB_TOKEN queues a PR, the queue's own checks never start. Commits use the no-reply address 66359507+HardMax71@users.noreply.github.com, set in this clone's git config.

Every pull request runs .github/workflows/gate.yml in a Debian 13 container with Rust 1.98.0, the same versions checks.yaml pins. On the pull request itself it only checks whitespace and validates the specs. The merge queue runs the real checks on the merged result in parallel jobs: lint (fmt, clippy, deny, shear, hooks and the other static checks, plus the gate's own tests when .development/gate changed), tests, and on a task branch a task job with the task's own commands and the oracle. Without a task, the queue keeps to the checks that don't depend on a base, since a queued group also holds the PRs ahead of it. A task run leaves out the mutants and coverage checks, which already passed on the branch and can take hours. The last job, gate, passes only when every part passed, and it is the check the merge queue requires. Pushes to main run lint and the whole test suite and refresh the caches.

On a fresh clone: open the folder in Claude Code and accept the workspace trust dialog, so the worker agents' Stop hook runs. Run `prek install --hook-type pre-commit`, so every commit in the clone and its worktrees runs the pre-commit hooks on its staged files. Plain `prek install` would also add the pre-push stage, which builds and tests the whole workspace on every push while the merge queue runs those checks anyway. Then run `cargo gate validate`. The dispatcher passes Codex `--dangerously-bypass-hook-trust`, since the only hook it runs is .codex/hooks.json from this repository. Both worker runners are sandboxed to their worktree plus the folders in agents.yaml policy.writable: the repository's .git folder (a worktree keeps its index, objects and refs there, and the gate keeps its journal there) and the cargo, prek and uv caches. Codex gets the list as writable_roots, and Claude through a settings file the dispatcher writes for each run, which runs every Bash command in Claude Code's sandbox. Everything else, ~/.cargo/bin included, stays read-only.

## Decisions

Max's product decisions so far. A task that contradicts one of them is wrong, not the decision.

- 2026-10-24: betterlite reimplements LiteLLM's proxy without its enterprise directory, from scratch but better, so wire names stay LiteLLM's: x-litellm-* response headers, litellm_params keys, LITELLM_* environment variables.
- 2026-10-24: config keys betterlite does not support yet stop startup with a list of them (B002).
- 2026-10-24: keys, spend and budgets (M4) reuse LiteLLM's Postgres schema, [schema.prisma](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/schema.prisma) at the pinned commit, so an existing LiteLLM database works with betterlite.
- 2026-10-24: the LICENSE names Max Azatian as copyright holder.
- 2026-10-24: the backlog covers every LiteLLM path outside the enterprise directory, and task ids run past B999 (B1000, B1001 and on) as the work needs.
- 2026-10-24: LiteLLM's Enterprise license check is left out. betterlite has no paid tier, so every feature is on and nothing caps users or teams.
- 2026-10-24: the admin UI is LiteLLM's MIT dashboard, kept, rebranded and served by betterlite (B144 to B150), not rewritten.
- 2026-10-24: LiteLLM's Terraform provider and VS Code extension are not rewritten. B155 and B156 make them work against betterlite.
- 2026-10-24: betterlite ships a Python package over the Rust core with LiteLLM's public API, used as `import betterlite as litellm`, and the proxy runs the Python callbacks and hooks config.yaml names (M12). The module is named betterlite because LiteLLM's name stays out of what betterlite ships.
- 2026-10-24: the rule questions R1 to R21 are answered. The answers live in rules.yaml, the clippy.toml and Cargo.toml templates, the no-loops-in-tests ast-grep rule and the 300-line source limit in checks.yaml.
- 2026-10-24: the pyramid questions are answered. Security tasks get an Opus review, Claude Opus gets one try after the Codex rungs, at most 3 dispatches run at once, every task gets a review, and `cargo gate plan` waits until 2 or 3 real dispatches show where the planner trips.
- 2026-10-24: the architect may raise a single task's budget (lines, expects, timeouts) when the work is cohesive and passes, and says why in the PR. Agents never get more access. B001 is the first: 1,700 lines for the oracle harness.
- 2026-10-24: up to 6 dispatches run at once, plus the planners, with a back-off when Codex returns rate limit errors. The merge queue squashes each pull request into one commit, so a branch that merged main lands without being rebuilt by hand. Content passes stay before a batch of tasks runs.
- 2026-10-24: the Codex usage limit ran out until 2026-10-30 22:51. Until then the worker ladder starts with the Claude Opus worker and Claude reviews Claude work (review_by_other_vendor off), with at most about 6 Claude agents at once. Revert both after the reset.
- 2026-10-24: shared list files are written once, not by tasks, so parallel task PRs never conflict and the merge queue can land them in any order. The oracle helper registry is pre-registered from every task file: one group file per crate lists its operations, each operation has its own module (a stub until its task builds it) and adapter, and HelperCall carries the operation by name. Task scopes list only their own files; a task that needs a new shared entry reports a blocker.
- 2026-10-24: a raised budget or widened scope ships in the task's own pull request (Max). A task branch may change its own task file and nothing else under .development/; the architect makes that commit, and the dispatcher sends back any worker run that touched .development/.
- 2026-10-25: Codex runs again ahead of its reset, so the worker ladder starts with the Codex rungs again and a reviewer from the other vendor reviews each run.
- 2026-10-25: the first worker rung (gpt-reserve, 3 attempts) runs at medium reasoning effort for speed (Max); the escalation rungs and every reviewer stay at high.
- 2026-10-27: cargo deny checks only the targets betterlite ships (x86_64 and aarch64 for linux-gnu, linux-musl and apple-darwin); a new target goes into deny.toml before a build for it ships.
- 2026-10-27: fuzz targets and corpora are always in scope, so any task may add a target and its seeds.
- 2026-10-27: the gate's tests check runs only the packages a change can affect (changed files, Cargo.lock and their dependents, from guppy's determinator); a push to main runs the whole suite, so a break that only shows through shared dependency features surfaces on main's run.
- 2026-10-28: the line budget leaves out test files (tests.rs and anything under a tests/ folder, the same files file_size holds to the test limit), since the coverage and mutants checks are what ask for those tests. Task sizes stay as planned (Max).
- 2026-10-29: the merge queue runs a task's checks with `--skip mutants --skip coverage`. The mutants check leaves out tools/oracle, since its tests start LiteLLM once per case and one oracle task needed hours of mutants. The local gate runs CI's `git diff --check` as the whitespace check. A Claude run that times out with empty stdout counts as no answer, so the ladder moves on. The worker report covers the whole branch and names each added crate exactly (Max).
- 2026-10-29: agents.yaml lists the vendors, each with the text its CLI prints at a usage limit. A run that ends without an answer and prints that text in its last lines marks the vendor, and every ladder skips that vendor's rungs for limit_retry_minutes (30). So Codex and Claude rungs share one ladder and nothing gets swapped by hand when a limit runs out. A task gets at most max_worker_attempts (6) worker runs across all rungs. The reviewer from the other vendor goes first, and one on the worker's vendor runs when every other one is at its limit. Each vendor runs at most max_parallel (8) agents at once unless its own entry sets another cap (Max).
- 2026-10-30: when every seat of a worker rung's vendor is taken, the try runs on the first later rung of another vendor that has a free seat and isn't at its limit. It still counts against its own rung. A worker run on a vendor with max_writers also takes one of those writer seats, so Claude works in at most 4 worktrees at once and its reviewers keep the other seats (Max).
- 2026-10-30: a Claude run that ends logged out ("Not logged in") or offline ("Can't reach the API server") marks the vendor the same way a usage limit does, since one login glitch on 10-29 cost 33 tries. Workers leave cargo alone while `cargo gate run` runs and never set CARGO_TARGET_DIR: 27 of the 30 Codex runs that timed out while still working had waited on that build lock, and 15 had rebuilt in a second target. The worker timeouts stay as they are (Max).

## Writing a task

The gate reads every format into Rust types that reject unknown fields, missing fields, duplicate keys, comments and anchors; only when_exists, link and the null ticket fields may be left out. Comments are rejected because an unquoted ` #` silently cuts the rest of a YAML value. On top of the types, validate checks what types can't: ids look like B004 or B1234, every DoD item names checks the task actually runs, every rule names real checks or is reviewed, runner arguments use only known placeholders, globs parse, links use https, dependencies exist and don't form a cycle, and every task file is in the backlog.

- Every DoD item names the checks that fail when the outcome is missing. A check is a shell command run from the repository root; exit 0 passes.
- A test check uses `cargo nextest run ... --no-tests=fail`, so a filter that matches nothing fails instead of passing.
- Behavior LiteLLM has belongs in oracle cases, not in prose.
- LiteLLM links point at the pinned commit d665dc1a6315847fcad6cc2c1264ceb47c06abce, never at main.
- Keep a task between 400 and 800 changed lines. Bigger work gets split.
- A task that changes the config, router or wire-format crates lists mutants in checks.optional, and a task that adds a parser of outside bytes adds its fuzz target and a test that replays the corpus.
- scope.paths lists exactly what the task may touch. Shared files like Cargo.toml and deny.toml go in only when the task needs them.

## Gate commands

| Command | What it does |
|---|---|
| `cargo gate validate` | Checks every spec file under .development/ and that the schemas are current |
| `cargo gate list <ID>` | Prints every check the task runs |
| `cargo gate run <ID>` | Runs them all and prints PASS, FAIL or SKIP per check; logs go to .git/betterlite/<ID>/gate/ |
| `cargo gate run <ID> --only <check>` | Runs one check (repeat --only for more) |
| `cargo gate dispatch <ID>` | Runs the task through the worker ladder and the reviewer; `--note "..."` (repeatable) hands the first attempt a finding the last run left open |
| `cargo gate schema` | Regenerates schema/ after a change to the gate's types |
| `cargo gate hook` | The Stop hook for Claude Code and Codex; reads the hook JSON on stdin |
| `cargo gate coverage` | Prints how many LiteLLM paths each backlog entry claims, then every unclaimed path and every glob that claims nothing |
| `cargo gate coverage --paths` | Prints every LiteLLM path with the backlog entries that cover it |
| `cargo gate show <ID>` | Shows an entry like a ticket: status and resolution, parent and other links, dependencies, what becomes ready once it is done, components (the crates in its scope), how many paths it covers, its dates from git, its merged size and its local dispatch attempts |
| `cargo gate inventory <checkout>` | Rewrites litellm-inventory.txt from a LiteLLM clone at the commit litellm.yaml pins |
| `cargo gate trace <path[:line]>` | Names the task that covers a LiteLLM path (with the pinned lines it ports and the betterlite files it writes) or that writes a betterlite file (with its LiteLLM sources), plus the deviations it logged |

The base is `GATE_BASE` when it is set, otherwise the merge base with origin/main, otherwise with main. Every command check gets `GATE_TASK`, `GATE_BASE`, `GATE_LOG_DIR` and `GATE_ORACLE_CASES` in its environment, and none of the variables cargo sets for the gate itself, so each check uses the toolchain its own worktree pins.

## LiteLLM at the pinned release

The oracle pins litellm[proxy] 1.105.1, released 2026-10-14 from [d665dc1a](https://github.com/BerriAI/litellm/tree/d665dc1a6315847fcad6cc2c1264ceb47c06abce). It was the newest release at least 7 days old on 2026-10-23. These numbers size the backlog:

| What | Count | Source |
|---|---|---|
| Providers | 184 | [provider_endpoints_support.json](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/provider_endpoints_support.json) |
| Providers with chat completions, responses, messages, embeddings | 142, 130, 128, 35 | same file |
| OpenAI-style routes | 127 | [_types.py#L339-L490](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L339-L490) |
| Anthropic-style routes | 9 | [_types.py#L551-L561](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L551-L561) |
| Config sections | 6 | [_types.py#L3241-L3272](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3241-L3272) |
| general_settings keys | 107 | [_types.py#L2698-L3238](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L2698-L3238) |
| router_settings keys | 19 | [router.py#L147-L172](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/router.py#L147-L172) |
| Routing strategies (default simple-shuffle) | 5 | [router.py#L866](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L866) |
| Database tables | 90 | [schema.prisma](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/schema.prisma) |
| Model price entries | 4460 | [model_prices_and_context_window.json](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/model_prices_and_context_window.json) |
| Lines in proxy_server.py and router.py | 20,379 and 14,759 | [proxy_server.py](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py), [router.py](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py) |
| Python lines in litellm/, without rust_bridge and the built UI | 884,838 | [litellm/](https://github.com/BerriAI/litellm/tree/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm) |
| Paths the backlog accounts for | 6,684 | litellm-inventory.txt |

The counts come from parsing those files at the pinned commit (json and ast for the Python, `wc -l` for line counts). The last row is why the file-size check exists.

## Why it's built this way

- A rule written in a prompt doesn't stop an agent; a check in code does. So every DoD item points at a command, every agent answer has a schema, and the Stop hook won't let an agent finish red.
- The mechanical middle of the pyramid (worktrees, attempts, escalation, the gate run, picking the reviewer) is the dispatcher, a program, because an orchestrating model forgets steps and a loop doesn't. Models do the parts that need judgment: planning, building, reviewing.
- The reviewer runs on another vendor than the worker whenever that vendor has usage left, so the two don't share the same blind spots.
- Anthropic's [harness for long-running agents](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents) kept a fixed feature list that agents could only mark as passing. Task files and the backlog are read-only for workers, and specs-untouched enforces it.
- Anthropic's [C compiler built by parallel agents](https://www.anthropic.com/engineering/building-c-compiler) used GCC as a known-good oracle and kept test output short with details in log files. The oracle harness and the gate's one-line-per-check output follow that.
- Cursor's [long-running agents](https://cursor.com/blog/scaling-agents) found locks and flat teams stall, while planners plus workers that grind on one task scale. The architect assigns tasks, so workers never claim or lock anything.
- OpenAI's [harness engineering](https://openai.com/index/harness-engineering/) kept AGENTS.md as a short map and enforced architecture mechanically. Crate layering lives in deny.toml wrappers and the rules live in rules.yaml.
- [SWE-bench](https://arxiv.org/abs/2310.06770) defines done as tests that go from failing to passing while the rest keep passing. Test checks with `--no-tests=fail`, plus the mutants check, carry that idea.
- Codex [subagents](https://developers.openai.com/codex/subagents) share one checkout, so parallel workers would collide. The dispatcher gives each task its own worktree instead.
