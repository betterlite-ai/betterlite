#!/usr/bin/env bash
source "${BETTERLITE_OPS:-/home/user/Desktop/betterlite/.development/ops}/common.sh"
# Dispatches one task and lands it. The gate keeps the task's progress in its journal until it merges, so this flow only pauses and reruns: a run that ends with every runner at its usage limit waits 30 minutes and reruns (uncapped, since a rerun against a marked vendor costs seconds), a run whose agent ended without an answer waits 20 minutes and reruns up to 4 times, and a stale branch gets main merged and one rerun. Any other error is a cap the architect handles. An existing branch gets main merged first; a ready branch gets main merged and must build, then is pushed, gets its PR with auto-merge and its done mark, and is watched until main's run on it passes. Usage: [GRANT=N] bash dispatch-flow.sh <id> <run> [note], where GRANT goes to the first dispatch only; LAND_ONLY=1 skips the dispatch for a branch that is already ready. Copied over dispatch-flow.sh once PR 388 is on main, so the feeder and the respawn scripts keep their names.
export GH_TOKEN="$(gh auth token --user HardMax71)"
S=/tmp/claude-1000/-home-user-Desktop-betterlite/b70e8a95-be32-4db9-80ae-d4296390a241/scratchpad
P=$S/prep
id=$1
run=$2
note=$3
grant=$GRANT
unset GRANT
mkdir -p "$L/locks"
exec 7> "$L/locks/$id.lock"
flock 7
other=$(cat "$L/locks/$id.pid" 2>/dev/null)
if [ -n "$other" ] && [ "$other" != "$$" ] && { tr "\\0" " " < "/proc/$other/cmdline"; } 2>/dev/null | grep -q "dispatch-flow.sh $id "; then echo "$(date +%H:%M) $id already has a flow, so $run did not start" >> "$L/flow-events.log"; exit 0; fi
echo $$ > "$L/locks/$id.pid"
exec 7>&-
worktree=$worktrees/$id
events=$L/flow-events.log
say() { echo "$(date +%H:%M) $id $*" >> "$events"; }
if [ -z "$LAND_ONLY" ]; then
while ! (cd "$main" && cargo gate availability) > "$L/availability-$id.log" 2>&1; do
  maintenance
  sleep 60
done
maintenance
say "dispatch $run"
if [ -d "$worktree" ]; then
  # a paused worker leaves uncommitted work, and git merge refuses to start over it; all worktrees share one stash list, so the entry is kept by its commit, never by position
  stashed=
  [ -n "$(git -C "$worktree" status --porcelain)" ] && stashed=$(flock "$L/locks/stash.lock" sh -c 'git -C "$1" stash push -u -q -m "$2" && git -C "$1" rev-parse refs/stash' sh "$worktree" "$id $run uncommitted work")
  bash "$S/merge-main.sh" "$worktree" | grep -q -E '^merged' || { say "blocked: main does not merge cleanly before $run${stashed:+ (uncommitted work kept in git stash $stashed)}"; exit 3; }
  if [ -n "$stashed" ]; then
    git -C "$worktree" stash apply -q "$stashed" || { say "blocked: the worker's uncommitted work conflicts with main after the merge (git stash $stashed)"; exit 3; }
    flock "$L/locks/stash.lock" sh -c 'n=$(git -C "$1" stash list --format=%H | grep -n -x -F "$2" | cut -d: -f1) && [ -n "$n" ] && git -C "$1" stash drop -q "stash@{$((n - 1))}"' sh "$worktree" "$stashed"
  fi
fi
GRANT=$grant bash "$ops/next-slot.sh" "$id" "$run" "$note" > /dev/null
code=$?
log=$L/dispatch-$id-$run.log
# the review's litellm findings: LiteLLM bugs the task repeats, for the architect
[ "$code" -eq 0 ] && grep -E "^$id repeats a LiteLLM bug" "$log" | cut -c1-600 | while read -r line; do say "${line#"$id "}"; done
if [ "$code" -ne 0 ]; then
  # the gate keeps progress.json in the journal until the task merges, so a rerun goes on from the same counts and needs no note again
  rm -rf "$main/.git/betterlite/$id"/mutants-work*
  if [ -z "$RETRIED" ] && grep -q -E 'does not match its format|merge conflict|behind main' "$log"; then
    bash "$S/merge-main.sh" "$worktree" | grep -q -E '^merged' || { say "blocked: main does not merge cleanly"; exit 3; }
    RETRIED=1 exec bash "$0" "$id" "${run}b" "main is merged into your branch now; continue where the last run stopped."
  fi
  if grep -q -E 'at its usage limit|every worker vendor is unavailable' "$log"; then
    say "paused: vendor unavailable; waiting for its reset or recovery before preparing another checkout"
    sleep 60
    maintenance
    LIMITS=$(( ${LIMITS:-0} + 1 )) BASE=${BASE:-$run} exec bash "$0" "$id" "${BASE:-$run}l$(( ${LIMITS:-0} + 1 ))"
  fi
  if grep -q -E 'ended without an answer|ended without a final answer' "$log" && ! grep -q "ladder ran out of attempts" "$log" && [ "${PAUSES:-0}" -lt 4 ]; then
    say "paused: an agent ended without an answer, retrying after a short recovery pause"
    sleep 60
    maintenance
    PAUSES=$(( ${PAUSES:-0} + 1 )) exec bash "$0" "$id" "${run}p"
  fi
  say "blocked: $(grep -m1 -o -E 'cannot dispatch [^:]+: .{0,300}' "$log")"
  # Preserve the build target and task temp files for a later explicit resume.
  exit 2
fi
fi
bash "$S/merge-main.sh" "$worktree" | grep -q -E '^merged' || { say "ready, but main does not merge cleanly"; exit 3; }
cd "$worktree" || exit 3
if ! built=$(bash "$P/land-check.sh" "$worktree" "$L/land-check-$id.log"); then
  say "ready, but the merged result failed to build: $built"
  exit 6
fi
exec >> "$L/land-$id.log" 2>&1
branch=$(git branch --show-current)
title=$(sed -n 's/^title: //p' "$main/.development/backlog/$id.yaml")
file=$(sed -n 's/^file: //p' "$main/.development/backlog/$id.yaml")
size=$(git diff --shortstat origin/main...HEAD -- . ':(exclude)Cargo.lock' | sed 's/^ //')
if ! pushed=$(git push -q origin "HEAD:refs/heads/$branch" 2>&1); then
  echo "$pushed"
  grep -q GH013 <<<"$pushed" && { say "ready, but GitHub push protection blocked the push, see logs/land-$id.log"; exit 5; }
  say "ready, but the PR did not open: the push failed, see logs/land-$id.log"; exit 4
fi
body=$L/pr-body-$id.md
printf 'Builds %s, from [its task file](https://github.com/betterlite-ai/betterlite/blob/%s/.development/%s). %s, Cargo.lock aside. The task gate passes and the review approved it. The PR also marks %s done in the backlog.\n' "$title" "$branch" "$file" "$size" "$id" > "$body"
old=$(git show "origin/main:.development/$file" | sed -n "s/^  max_changed_lines: //p")
new=$(sed -n "s/^  max_changed_lines: //p" ".development/$file")
if [ "$old" != "$new" ]; then
  printf "\nThe architect raised its line budget from %s to %s: %s\n" "$old" "$new" "$(git log -1 --format=%b --grep="^docs(tasks): $id budget " origin/main..HEAD | head -n 1)" >> "$body"
fi
git log --reverse --format=%H --grep="^docs(tasks): $id scope " origin/main..HEAD | while read -r c; do
  printf "\nThe architect added %s to its scope: %s\n" "$(git log -1 --format=%s "$c" | sed "s|^docs(tasks): $id scope ||")" "$(git log -1 --format=%b "$c" | head -n 1)" >> "$body"
done
git log --reverse --format=%H --grep="^docs(tasks): $id " origin/main..HEAD | while read -r c; do
  case "$(git log -1 --format=%s "$c")" in "docs(tasks): $id budget "*|"docs(tasks): $id scope "*) continue ;; esac
  printf "\nThe architect changed its task file: %s\n" "$(git log -1 --format=%b "$c" | head -n 1)" >> "$body"
done
pr=$(gh pr list --head "$branch" --state open --json number -q ".[0].number")
[ -n "$pr" ] || gh pr create --base main --head "$branch" --title "$id: $title" --body-file "$body" > /dev/null || { say "ready, but the PR did not open"; exit 4; }
for try in 1 2 3; do pr=$(gh pr view "$branch" --json number -q .number 2>/dev/null); [ -n "$pr" ] && break; sleep 10; done
[ -n "$pr" ] || { say "ready, but the PR did not open"; exit 4; }
sed -i "s/^status: .*/status: done/; s/^resolution: .*/resolution: merged/; s/^pr: .*/pr: $pr/" ".development/backlog/$id.yaml"
if ! git diff --quiet -- ".development/backlog/$id.yaml"; then
  git add ".development/backlog/$id.yaml" && git commit -q -m "docs(backlog): $id done"
  if ! pushed=$(git push -q origin "HEAD:refs/heads/$branch" 2>&1); then
    echo "$pushed"
    say "PR #$pr open, but the done-mark push failed, see logs/land-$id.log"; exit 4
  fi
fi
gh pr merge "$branch" --auto --squash > /dev/null 2>&1
say "PR #$pr open: https://github.com/betterlite-ai/betterlite/pull/$pr"
verified_head=$(git rev-parse HEAD) || exit 5
result=$(bash "$ops/pr-watch.sh" "$pr")
say "$result"
case "$result" in
  *"merged as"*passed)
    cd "$main" || exit 5
    if cleanup=$(uv run --locked --project "$ops" python "$ops/cleanup.py" "$main" "$worktree" "$verified_head" 2>&1); then
      say "$cleanup"
    else
      say "merged checkout retained: $cleanup"
    fi
    exit 0 ;;
  *) exit 5 ;;
esac
