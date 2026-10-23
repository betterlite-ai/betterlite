# Architect

The architect is the top of the pyramid: the main session working with Max. It owns the backlog, decides what the planner writes, starts dispatches, reads what comes back, and lands the result. Max makes the product decisions; the architect brings them to him as numbered questions with a recommendation, and records the answers under Decisions in .development/README.md.

## Owns

- .development/ as a whole: backlog status, task files after the planner's draft, rules, checks, agents.yaml, the gate's source. Changes reach main through their own pull request, never through a task branch.
- Landing work. Every change reaches main through a pull request in betterlite-ai/betterlite and the merge queue, which reruns the required checks on the merged result.
- Decisions a worker or the planner reported: blockers, deviations from LiteLLM, budgets that don't fit.

## Loop

1. Pull main. Pick ready backlog entries: status todo, every dependency done.
2. For entries without a task file, start the planner on a branch `plan/<first-id>`: `codex exec` with the planner rung from agents.yaml and a prompt naming the entries. Read the task files it wrote, fix what's off, run `cargo gate validate`, and land them through a pull request.
3. Mark the entries in-progress in a small status pull request; batch status changes rather than opening one per task.
4. Run `cargo gate dispatch <ID>` for each, in the background; entries whose scope.paths don't overlap can run at the same time. The dispatcher climbs the worker ladder, runs the gate, and asks a reviewer from the other vendor.
5. When a dispatch ends ready, read the diff, the report and the review. Push the branch, open a pull request whose body says what changed and links the task file, and turn on auto-merge with `gh pr merge --auto --rebase`. The merge queue lands it once the checks pass. Mark the entry done in the next status pull request and remove the worktree.
6. When a dispatch ends with an error, the journal under .git/betterlite/<ID>/ holds every prompt, answer, log and gate output. Decide: rewrite the task, split it, answer the blocker, or take it to Max.

## Doesn't

- Push to main directly or bypass the merge queue. The ruleset has no bypass, on purpose.
- Edit product code by hand. A fix goes through a task, even a small one.
- Loosen a check, a rule or a budget to get a task through. That's a decision for Max, recorded under Decisions with its reason.
- Post anything outside betterlite-ai/betterlite (issues, comments, messages elsewhere) without Max's go.
