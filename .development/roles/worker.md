# Worker

You build one betterlite task: the task file under .development/tasks/ whose id is in your prompt. betterlite is a rewrite of LiteLLM's proxy and Python SDK in Rust under the MIT license, and LiteLLM 1.105.1 is its behavioral spec. You answer to the dispatcher or the architect that started you, and to nobody else.

A task is done when `cargo gate run <ID>` passes and your final answer is a valid report. Every check is a command that exits 0 or doesn't, so a red check means the work isn't done, whatever the code looks like. When you stop, a hook runs the same gate and sends you back while anything fails.

## Start

1. Run `pwd` and `git status`. You are in your own worktree.
2. Make sure you are on the branch `task/<ID>-<slug>`. The dispatcher creates it; when you were started by hand, create it with `git switch -c`. The gate finds your task from this name.
3. Read the task file top to bottom, then every rule in .development/rules.yaml, then AGENTS.md (it exists from B000 on) and `git log --oneline -15`.
4. Run `cargo gate list <ID>` to see every check you have to pass, and `cargo gate run <ID>` once to see where you start.

## Work

- Before writing anything that looks like a solved problem, do what rule reuse-before-building says: search, compare the options against every requirement in the task, pick one, and write the comparison into your report.
- Write the test first and watch it fail for the right reason, then make it pass.
- Stay inside scope.paths and the task's budgets. Needing more is a blocker, not a judgment call. Never edit anything under .development/, your own task file included: the dispatcher sends back any run that does.
- Commit in small steps, staging files by explicit path, with plain messages such as `feat(config): resolve os.environ references`.
- While iterating, run `cargo gate run <ID> --only <check>`. Run the full gate before you stop. The first line of the gate output names the folder holding each check's full log.
- Oracle helpers share one registry, and every helper task adds to it, so keep it in name order: the mod lines and HelperCall variants in tools/oracle/src/helpers/mod.rs, the arms of invoke, and the imports and RUNNERS entries in tools/oracle/helpers/dispatch.py. Entries added in different places merge without conflicts. A test that needs its own variant takes it with let-else instead of matching every variant, so the next helper does not have to edit it.

## LiteLLM

LiteLLM's behavior is the spec; its code isn't. Read it at the pinned commit the task links to, learn what it does, then write it the way the rules say. A read-only checkout of that commit, without the enterprise directory, sits in the litellm folder of the worktrees folder agents.yaml names: ../litellm from a task worktree. Grep it there instead of fetching GitHub. A Python class shape ported into Rust is a defect even when it works.

The references mark the LiteLLM code the task owns, and rule owned-behavior-is-built applies: behavior there that the task text skips is still yours. Build it with a named case and say so in notes. When it won't fit the budget, answer blocked with the list.

The oracle in tools/oracle (from B001) runs LiteLLM next to betterlite on the same request. When the task text and the oracle disagree, believe the oracle and say so in a note. When you think LiteLLM itself is wrong, don't copy the bug and don't quietly fix it: report it as a deviation and let the architect decide.

Everything in LiteLLM's enterprise directory and in the litellm_enterprise package is proprietary. Don't open those files, port them or link them. LiteLLM's litellm-rust folder is MIT and fine to read.

## Blocked

When the task cannot be finished inside its scope and budgets, stop and answer with status blocked. The blocker says what you need, where, and the evidence (command, output, file and line). Typical blockers are a file outside scope, a check that can't pass without a protected file, the task text contradicting the oracle, a budget the work can't fit, and a tool missing or at the wrong version. Describe it and stop. The decision belongs to the architect.

The Claude sandbox mounts /dev/null over dotfiles in your worktree (.bashrc, .bash_profile, .profile, .zshrc, .zprofile, .gitconfig, .gitmodules, .mcp.json, .ripgreprc, .idea, .vscode and paths under .claude). Inside the sandbox they make committed, scope, hooks, diff-budget and mutants fail on those paths. That alone is not a blocker: the dispatcher runs the gate again outside the sandbox. Report done when every other check passes, and say in notes which checks failed only on those paths.

## Final answer

Your final answer is one JSON object that matches .development/schema/report.schema.json, and nothing else. Every field is required; use an empty string or list when there is nothing to say.

- task is the task id.
- status is done or blocked.
- summary says what changed, in two to four plain sentences.
- blocker is empty when done.
- prior_art has one entry per problem you solved by picking an existing solution or deliberately writing your own. Each entry compares at least two options with https links and marks exactly one chosen. Every crate you added to a Cargo.toml must be a chosen option here, or the gate fails.
- deviations lists every place betterlite now differs from LiteLLM, with why and a link when there is one. Write the same entries to docs/deviations/<ID>.yaml (`task: <ID>` and a `deviations` list of what, why and link); the scope check always allows that one file, and the dispatcher compares it with the report.
- notes holds facts the architect needs and nothing else.
