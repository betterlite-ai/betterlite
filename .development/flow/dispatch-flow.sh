#!/usr/bin/env bash
P=$(dirname "$(readlink -f "$0")")
main=${BETTERLITE_ROOT:-$(git -C "$P" rev-parse --show-toplevel)}
worktrees=${BETTERLITE_WORKTREES:-$(dirname "$main")/betterlite-worktrees}
L=$worktrees/logs
maintenance() {
  if [ -e "$main/.git/betterlite/maintenance" ] || [ -e "$L/maintenance.STOP" ]; then
    echo "maintenance is active; no dispatch or background build started" >&2
    trap - EXIT
    exit 75
  fi
}
maintenance
# Dispatches one task and lands it. The gate keeps the task's progress in its journal until it merges, so this flow only pauses and reruns: a run that ends with every runner at its usage limit waits 30 minutes and reruns (uncapped, since a rerun against a marked vendor costs seconds), a run whose agent ended without an answer waits 20 minutes and reruns up to 4 times, and a stale branch gets main merged and one rerun. Any other error is a cap the architect handles. An existing branch gets main merged first; a ready branch gets main merged, then is pushed, gets its PR with auto-merge and its done mark, and is watched until main's run on it finishes; a red CI run on the PR goes back to the worker as a fix round with its FAIL lines, three rounds at most; once it merged, its checkout (target included) and temp folder are removed whatever main's run says, since refs/betterlite/completed keeps its head. Usage: [GRANT=N] bash dispatch-flow.sh <id> <run> [note], where GRANT goes to the first dispatch only; LAND_ONLY=1 skips the dispatch for a branch that is already ready. Copied over dispatch-flow.sh once PR 388 is on main, so the feeder and the respawn scripts keep their names.
export GH_TOKEN="$(gh auth token --user HardMax71)"
id=$1
[[ $id =~ ^B[0-9]+$ ]] || exit 2
if [ -n "${WAIT_FOR:-}" ]; then
  [[ $WAIT_FOR =~ ^B[0-9]+$ ]] || exit 2
  echo "$(date +%H:%M) $id waiting for $WAIT_FOR to merge before dispatch" >> "$L/flow-events.log"
  until grep -q '^status: done$' "$main/.development/backlog/$WAIT_FOR.yaml"; do
    maintenance
    sleep 60
  done
  unset WAIT_FOR
fi
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
# what the feeder needs to bring this flow back if the machine or the flow dies; a flow that ends on its own removes it
mkdir -p "$L/flows"
args=$L/flows/$id.args
printf 'run=%q\nnote=%q\ngrant=%q\nci_rounds=%q\nconflicts=%q\nland_only=%q\n' "$run" "$note" "$grant" "${CI_ROUNDS:-0}" "${CONFLICTS:-0}" "${LAND_ONLY:-}" > "$args"
trap 'rm -f -- "$args"' EXIT
worktree=$worktrees/$id
events=$L/flow-events.log
say() { echo "$(date +%H:%M) $id $*" >> "$events"; }
park() { say "blocked: $*"; printf '%s\t%s\t%s\t%s\t%s\n' "$(date '+%F %H:%M')" "$id" "$run" "$main/.git/betterlite/$id" "$(tr '\t\n' '  ' <<< "$*")" >> "$L/parked.tsv"; }
if [ -z "$LAND_ONLY" ]; then
while ! (cd "$main" && cargo gate availability) > "$L/availability-$id.log" 2>&1; do
  maintenance
  sleep 60
done
maintenance
say "dispatch $run"
if [ -d "$worktree" ]; then
  # an agent sandbox that dies leaves its empty read-only .git/config.lock, and the gate then fails at remote set-url; no agent runs in this tree before next-slot
  [ -f "$worktree/.git/config.lock" ] && [ ! -s "$worktree/.git/config.lock" ] && rm -f -- "$worktree/.git/config.lock" && say "removed a stale empty .git/config.lock"
  # a paused worker leaves uncommitted work, and git merge refuses to start over it; all worktrees share one stash list, so the entry is kept by its commit, never by position
  stashed=
  [ -n "$(git -C "$worktree" status --porcelain)" ] && stashed=$(flock "$L/locks/stash.lock" sh -c 'git -C "$1" stash push -u -q -m "$2" && git -C "$1" rev-parse refs/stash' sh "$worktree" "$id $run uncommitted work")
  bash "$P/merge-main.sh" "$worktree" | grep -q -E '^merged' || { park "main does not merge cleanly before $run${stashed:+ (uncommitted work kept in git stash $stashed)}"; exit 3; }
  if [ -n "$stashed" ]; then
    git -C "$worktree" stash apply -q "$stashed" || { park "the worker's uncommitted work conflicts with main after the merge (git stash $stashed)"; exit 3; }
    flock "$L/locks/stash.lock" sh -c 'n=$(git -C "$1" stash list --format=%H | grep -n -x -F "$2" | cut -d: -f1) && [ -n "$n" ] && git -C "$1" stash drop -q "stash@{$((n - 1))}"' sh "$worktree" "$stashed"
  fi
fi
GRANT=$grant bash "$P/next-slot.sh" "$id" "$run" "$note" > /dev/null
code=$?
# maintenance stopped the run: the flow comes back when it ends
[ "$code" -eq 75 ] && { trap - EXIT; exit 75; }
log=$L/dispatch-$id-$run.log
# the review's litellm findings: LiteLLM bugs the task repeats, for the architect
[ "$code" -eq 0 ] && grep -E "^$id repeats a LiteLLM bug" "$log" | cut -c1-600 | while read -r line; do say "${line#"$id "}"; done
if [ "$code" -ne 0 ]; then
  # the gate keeps progress.json in the journal until the task merges, so a rerun goes on from the same counts and needs no note again
  rm -rf "$main/.git/betterlite/$id"/mutants-work*
  if [ -z "$RETRIED" ] && grep -q -E 'does not match its format|merge conflict|behind main' "$log"; then
    bash "$P/merge-main.sh" "$worktree" | grep -q -E '^merged' || { park "main does not merge cleanly"; exit 3; }
    RETRIED=1 exec bash "$0" "$id" "${run}b" "main is merged into your branch now; continue where the last run stopped."
  fi
  if grep -q -E 'at its usage limit|ladder is unavailable|every worker vendor is unavailable' "$log"; then
    why=$(grep -o -E "[a-z]+ (can't reach its API|is logged out|is at its usage limit)" "$log" | sort -u | paste -sd, -)
    say "paused: ${why:-vendor unavailable}; waiting for its reset or recovery before preparing another checkout"
    sleep 60
    maintenance
    LIMITS=$(( ${LIMITS:-0} + 1 )) BASE=${BASE:-$run} exec bash "$0" "$id" "${BASE:-$run}l$(( ${LIMITS:-0} + 1 ))" "$note"
  fi
  if grep -q -E 'ended without an answer|ended without a final answer' "$log" && ! grep -q "ladder ran out of attempts" "$log" && [ "${PAUSES:-0}" -lt 4 ]; then
    say "paused: an agent ended without an answer, retrying after a short recovery pause"
    sleep 60
    maintenance
    PAUSES=$(( ${PAUSES:-0} + 1 )) exec bash "$0" "$id" "${run}p" "$note"
  fi
  reason=$(grep -m1 -o -E 'cannot dispatch [^:]+: .*' "$log")
  if [ -n "$reason" ] && [ -d "$worktree" ] && unblock_note=$(bash "$P/unblock.sh" "$id" "$worktree" "$reason"); then
    say "unblocked by a rule: $unblock_note"
    GRANT=1 exec bash "$0" "$id" "${run}u" "$unblock_note"
  fi
  park "${reason:0:300}"
  # Preserve the build target and task temp files for a later explicit resume.
  exit 2
fi
fi
sed -i 's/^land_only=.*/land_only=1/' "$args"
bash "$P/merge-main.sh" "$worktree" | grep -q -E '^merged' || { say "ready, but main does not merge cleanly"; exit 3; }
cd "$worktree" || exit 3
exec >>"$L/land-$id.log" 2>&1
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
printf 'Builds %s, from [its task file](https://github.com/betterlite-ai/betterlite/blob/%s/.development/%s). %s, Cargo.lock aside. The lint checks pass locally and the review approved it. CI runs the tests, the oracle cases and mutants. The PR also marks %s done in the backlog.\n' "$title" "$branch" "$file" "$size" "$id" > "$body"
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
[ -n "$pr" ] || gh pr create --base main --head "$branch" --title "$id: $title" --body-file "$body" ${PR_LABEL:+--label "$PR_LABEL"} > /dev/null || { say "ready, but the PR did not open"; exit 4; }
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
result=$(bash "$P/pr-watch.sh" "$pr")
say "${result:-pr-watch for PR #$pr ended without a result; checkout kept}"
case "$result" in
  *"merged as"*)
    cd "$main" || exit 5
    if ! status=$(git -C "$worktree" status --porcelain) || [ -n "$status" ] ||
       [ "$(git -C "$worktree" branch --show-current)" != "$branch" ] ||
       [ "$(git -C "$worktree" rev-parse HEAD)" != "$verified_head" ] ||
       find /proc/[0-9]*/cwd -maxdepth 0 \( -lname "$worktree" -o -lname "$worktree/*" \) 2>/dev/null | grep -q .; then
      say "merged checkout retained: changed files or a live process"
    elif git fetch --no-tags "$worktree" "HEAD:refs/betterlite/completed/$id" &&
         [ "$(git rev-parse "refs/betterlite/completed/$id")" = "$verified_head" ] &&
         [ "$(git -C "$worktree" rev-parse HEAD)" = "$verified_head" ] &&
         status=$(git -C "$worktree" status --porcelain) && [ -z "$status" ] &&
         ! find /proc/[0-9]*/cwd -maxdepth 0 \( -lname "$worktree" -o -lname "$worktree/*" \) 2>/dev/null | grep -q .; then
      if [ -f "$worktree/.git" ]; then
        git worktree remove "$worktree" || exit 5
      else
        rm -rf -- "$worktree"
      fi
      rm -rf -- "$worktrees/tmp/$id" "$main/.git/betterlite/$id"/mutants-work*
      say "archived $verified_head; removed merged checkout and its temp folder"
    else
      say "merged checkout retained: archival or final idle check failed"
    fi
    case "$result" in *passed) exit 0 ;; *) exit 5 ;; esac ;;
  # CI runs every check that builds tests, so a red run goes back to the worker with its gate lines and logs, three rounds at most
  *" run "*" failed"*)
    run_id=$(grep -o -E 'run [0-9]+ failed' <<< "$result" | head -1 | tr -dc 0-9)
    [ "${CI_ROUNDS:-0}" -lt 3 ] || { park "CI failed on PR #$pr (run $run_id) after 3 fix rounds"; exit 5; }
    rm -rf -- "$worktree/target/ci-logs"
    logs=$worktree/target/ci-logs/$run_id
    gh run download "$run_id" -D "$logs" > /dev/null 2>&1
    failed_log=$(gh run view "$run_id" --log-failed 2>/dev/null | awk -F'\t' '$1 != "gate"' | cut -f3- | sed -E 's/^\xEF\xBB\xBF//; s/^[0-9TZ:.-]+ //; s/(\x1b|\^\[)\[[0-9;]*m//g' | grep -v '^##\[')
    lines=$(grep -E '^FAIL |^     ' <<< "$failed_log" | head -n 60)
    # a step outside the gate (the gate's own tests, a tool install) prints no FAIL line, so its last lines go instead
    [ -n "$lines" ] || lines=$(tail -n 40 <<< "$failed_log")
    missed=$(find "$logs" -path '*/mutants.out/missed.txt' -exec cat {} + 2>/dev/null | sort -u | head -n 40)
    timed_out=$(find "$logs" -path '*/mutants.out/timeout.txt' -exec cat {} + 2>/dev/null | sort -u | head -n 20)
    say "CI failed on PR #$pr (run $run_id); fix round $(( ${CI_ROUNDS:-0} + 1 ))"
    LAND_ONLY= CI_ROUNDS=$(( ${CI_ROUNDS:-0} + 1 )) exec bash "$0" "$id" "${run}c" "CI run https://github.com/betterlite-ai/betterlite/actions/runs/$run_id failed on PR #$pr.
$lines${missed:+
Mutants CI missed:
$missed}${timed_out:+
Mutants that timed out in CI (the gate counts a timeout as a failure, so a test must fail on the mutant instead of hanging):
$timed_out}
The gate logs of that run are in target/ci-logs/$run_id/ in your worktree. Fix the cause and finish as usual. The final gate here runs only the lint checks; CI runs the tests again once the branch is pushed." ;;
  # main moved under the open PR; the land path merges main again (line 88) and stops when the merge needs a person, three rounds at most
  *" conflicts with main"*)
    [ "${CONFLICTS:-0}" -lt 3 ] || { park "PR #$pr conflicts with main after 3 merges of main"; exit 5; }
    say "PR #$pr conflicts with main; merging main and landing again"
    LAND_ONLY=1 CONFLICTS=$(( ${CONFLICTS:-0} + 1 )) exec bash "$0" "$id" "${run}m" "" ;;
  *) exit 5 ;;
esac
