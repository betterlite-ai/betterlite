#!/usr/bin/env bash
self=$(readlink -f "$0") || exit 1
P=${self%/*}
main=${BETTERLITE_ROOT:-$(git -C "${P}" rev-parse --show-toplevel)}
worktrees=${BETTERLITE_WORKTREES:-$(dirname "${main}")/betterlite-worktrees}
L=${worktrees}/logs
maintenance() {
  if [[ -e "${main}/.git/betterlite/maintenance" ]] || [[ -e "${L}/maintenance.STOP" ]]; then
    echo "maintenance is active; no dispatch or background build started" >&2
    trap - EXIT
    exit 75
  fi
}
maintenance
# Dispatches one task and lands it. The gate keeps the task's progress in its journal until it merges, so this flow only pauses and reruns: a run that ends with every runner at its usage limit waits 30 minutes and reruns (uncapped, since a rerun against a marked vendor costs seconds), a run whose agent ended without an answer waits 20 minutes and reruns up to 4 times, and a stale branch gets main merged and one rerun. Any other error is a cap the architect handles. An existing branch gets main merged first; a ready branch gets main merged and its early draft PR's branch (early-pr.sh) merged back, then is pushed, gets its PR with auto-merge and its done mark (a push or PR that GitHub refuses lands again after 5 minutes, three times at most), and is watched until it merges; a red CI run on the PR goes back to the worker as a fix round with its FAIL lines, eight rounds at most (stage 3, since CI is the only place that builds); once it merged, its checkout (target included) and temp folder are removed right away, since refs/betterlite/completed keeps its head, and the flow then waits for main's run on it to log the result. Usage: [GRANT=N] bash dispatch-flow.sh <id> <run> [note], where GRANT goes to the first dispatch only; LAND_ONLY=1 skips the dispatch for a branch that is already ready. Copied over dispatch-flow.sh once PR 388 is on main, so the feeder and the respawn scripts keep their names.
GH_TOKEN=$(gh auth token --user HardMax71)
export GH_TOKEN
id=$1
[[ ${id} =~ ^B[0-9]+$ ]] || exit 2
if [[ -n "${WAIT_FOR:-}" ]]; then
  [[ ${WAIT_FOR} =~ ^B[0-9]+$ ]] || exit 2
  printf '%(%H:%M)T %s\n' -1 "${id} waiting for ${WAIT_FOR} to merge before dispatch" >> "${L}/flow-events.log"
  until grep -q '^status: done$' "${main}/.development/backlog/${WAIT_FOR}.yaml"; do
    maintenance
    sleep 60
  done
  unset WAIT_FOR
fi
run=$2
note=$3
grant=${GRANT:-}
unset GRANT
mkdir -p "${L}/locks"
exec 7> "${L}/locks/${id}.lock"
flock 7
other=$(cat "${L}/locks/${id}.pid" 2> /dev/null)
if [[ -n "${other}" ]] && [[ "${other}" != "$$" ]] && cmdline=$(tr '\0' ' ' 2> /dev/null < "/proc/${other}/cmdline") && grep -q "dispatch-flow.sh ${id} " <<< "${cmdline}"; then
  printf '%(%H:%M)T %s\n' -1 "${id} already has a flow, so ${run} did not start" >> "${L}/flow-events.log"
  exit 0
fi
echo $$ > "${L}/locks/${id}.pid"
exec 7>&-
# a measured trial gives the next first dispatches the note in logs/trial-note.txt while logs/trial-slots counts above 0
if [[ "${run}" = run1 ]] && [[ -z "${note}" ]] && [[ -z "${LAND_ONLY:-}" ]] && [[ -s "${L}/trial-note.txt" ]] &&
  { flock 8 && slots=$(cat "${L}/trial-slots" 2> /dev/null) && [[ "${slots:-0}" -gt 0 ]] && echo $((slots - 1)) > "${L}/trial-slots"; } 8> "${L}/locks/trial.lock"; then
  note=$(cat "${L}/trial-note.txt")
  printf '%(%H:%M)T %s\n' -1 "${id} trial note for ${run}" >> "${L}/flow-events.log"
fi
# what the feeder needs to bring this flow back if the machine or the flow dies; a flow that ends on its own removes it
mkdir -p "${L}/flows"
args=${L}/flows/${id}.args
jq -n --arg run "${run}" --arg note "${note}" --arg grant "${grant}" --arg ci_rounds "${CI_ROUNDS:-0}" --arg conflicts "${CONFLICTS:-0}" --arg land_only "${LAND_ONLY:-}" '$ARGS.named' > "${args}"
trap 'rm -f -- "$args"' EXIT
worktree=${worktrees}/${id}
events=${L}/flow-events.log
say() { printf '%(%H:%M)T %s\n' -1 "${id} $*" >> "${events}"; }
park() {
  say "blocked: $*"
  # the reason column ends with a space, as every earlier parked.tsv row does
  printf '%(%F %H:%M)T\t%s\t%s\t%s\t%s \n' -1 "${id}" "${run}" "${main}/.git/betterlite/${id}" "${*//[$'\t\n']/ }" >> "${L}/parked.tsv"
}
# GitHub refuses a push or a new PR now and then with a short-lived error (HTTP 408, a bare remote rejected, a TLS timeout), so a ready branch lands again
land_again() {
  [[ "${LANDS:-0}" -lt 3 ]] || {
    say "$1"
    exit 4
  }
  say "$1; landing again in 5 minutes"
  sleep 300
  LANDS=$((${LANDS:-0} + 1)) LAND_ONLY=1 exec bash "${P}/dispatch-flow.sh" "${id}" "${run}" ""
}
if [[ -z "${LAND_ONLY}" ]]; then
  while ! (cd "${main}" && cargo gate availability) > "${L}/availability-${id}.log" 2>&1; do
    maintenance
    sleep 60
  done
  maintenance
  say "dispatch ${run}"
  if [[ -d "${worktree}" ]]; then
    # an agent sandbox that dies leaves its empty read-only .git/config.lock, and the gate then fails at remote set-url; no agent runs in this tree before next-slot
    [[ -f "${worktree}/.git/config.lock" ]] && [[ ! -s "${worktree}/.git/config.lock" ]] && rm -f -- "${worktree}/.git/config.lock" && say "removed a stale empty .git/config.lock"
    # a paused worker leaves uncommitted work, and git merge refuses to start over it; all worktrees share one stash list, so the entry is kept by its commit, never by position
    stashed=
    changes=$(git -C "${worktree}" status --porcelain) && [[ -n "${changes}" ]] && stashed=$({ flock 8 && git -C "${worktree}" stash push -u -q -m "${id} ${run} uncommitted work" && git -C "${worktree}" rev-parse refs/stash; } 8> "${L}/locks/stash.lock")
    merge=$(bash "${P}/merge-main.sh" "${worktree}")
    grep -q -E '^merged' <<< "${merge}" || {
      park "main does not merge cleanly before ${run}${stashed:+ (uncommitted work kept in git stash ${stashed})}"
      exit 3
    }
    if [[ -n "${stashed}" ]]; then
      git -C "${worktree}" stash apply -q "${stashed}" || {
        park "the worker's uncommitted work conflicts with main after the merge (git stash ${stashed})"
        exit 3
      }
      { flock 8 && stashes=$(git -C "${worktree}" stash list --format=%H) && entry=$(grep -n -x -F "${stashed}" <<< "${stashes}") && git -C "${worktree}" stash drop -q "stash@{$((${entry%%:*} - 1))}"; } 8> "${L}/locks/stash.lock"
    fi
    # a crate loop with main or a running branch goes back to the worker with the report and the scope to move the code, three rounds at most
    if ! bash "${P}/crate-cycles.sh" "${worktree}" > /dev/null; then
      note=$(bash "${P}/unblock.sh" "${id}" "${worktree}" "crate cycle") || {
        cycle=$(bash "${P}/crate-cycles.sh" "${worktree}")
        park "crate cycle no rule could break: ${cycle%%$'\n'*}"
        exit 3
      }
      grant=1
      say "crate cycle, back to the worker: ${note%%$'\n'*}"
    fi
  fi
  # the review gets a draft PR, so CI runs while it reviews
  bash "${P}/early-pr.sh" "${id}" "$$" >> "${L}/early-pr-${id}.log" 2>&1 &
  early=$!
  GRANT=${grant} bash "${P}/next-slot.sh" "${id}" "${run}" "${note}" > /dev/null
  code=$?
  kill "${early}" 2> /dev/null
  # maintenance stopped the run: the flow comes back when it ends
  [[ "${code}" -eq 75 ]] && {
    trap - EXIT
    exit 75
  }
  log=${L}/dispatch-${id}-${run}.log
  # the review's litellm findings: LiteLLM bugs the task repeats, for the architect
  if [[ "${code}" -eq 0 ]] && repeats=$(grep -E "^${id} repeats a LiteLLM bug" "${log}"); then
    repeats=$(cut -c1-600 <<< "${repeats}")
    while read -r line; do say "${line#"${id} "}"; done <<< "${repeats}"
  fi
  if [[ "${code}" -ne 0 ]]; then
    # the gate keeps progress.json in the journal until the task merges, so a rerun goes on from the same counts and needs no note again
    rm -rf "${main}/.git/betterlite/${id}"/mutants-work*
    if [[ -z "${RETRIED}" ]] && grep -q -E 'does not match its format|merge conflict|behind main' "${log}"; then
      merge=$(bash "${P}/merge-main.sh" "${worktree}")
      grep -q -E '^merged' <<< "${merge}" || {
        park "main does not merge cleanly"
        exit 3
      }
      RETRIED=1 exec bash "${P}/dispatch-flow.sh" "${id}" "${run}b" "main is merged into your branch now; continue where the last run stopped."
    fi
    if grep -q -E 'at its usage limit|ladder is unavailable|every worker vendor is unavailable' "${log}"; then
      why=$(grep -o -E "[a-z]+ (can't reach its API|is logged out|is at its usage limit)" "${log}") && why=$(sort -u <<< "${why}") && why=${why//$'\n'/,}
      say "paused: ${why:-vendor unavailable}; waiting for its reset or recovery before preparing another checkout"
      sleep 60
      maintenance
      LIMITS=$((${LIMITS:-0} + 1))
      BASE=${BASE:-${run}}
      export LIMITS BASE
      exec bash "${P}/dispatch-flow.sh" "${id}" "${BASE}l${LIMITS}" "${note}"
    fi
    if grep -q -E 'ended without an answer|ended without a final answer' "${log}" && ! grep -q "ladder ran out of attempts" "${log}" && [[ "${PAUSES:-0}" -lt 4 ]]; then
      say "paused: an agent ended without an answer, retrying after a short recovery pause"
      sleep 60
      maintenance
      PAUSES=$((${PAUSES:-0} + 1)) exec bash "${P}/dispatch-flow.sh" "${id}" "${run}p" "${note}"
    fi
    reason=$(grep -m1 -o -E 'cannot dispatch [^:]+: .*' "${log}")
    if [[ -n "${reason}" ]] && [[ -d "${worktree}" ]] && unblock_note=$(bash "${P}/unblock.sh" "${id}" "${worktree}" "${reason}"); then
      say "unblocked by a rule: ${unblock_note}"
      GRANT=1 exec bash "${P}/dispatch-flow.sh" "${id}" "${run}u" "${unblock_note}"
    fi
    park "${reason:0:300}"
    # Preserve the build target and task temp files for a later explicit resume.
    exit 2
  fi
fi
landing=$(jq '.land_only = "1"' "${args}") && printf '%s\n' "${landing}" > "${args}"
merge=$(bash "${P}/merge-main.sh" "${worktree}")
grep -q -E '^merged' <<< "${merge}" || {
  park "ready, but main does not merge cleanly"
  exit 3
}
if ! bash "${P}/crate-cycles.sh" "${worktree}" > /dev/null; then LAND_ONLY='' exec bash "${P}/dispatch-flow.sh" "${id}" "${run}d" ""; fi
cd "${worktree}" || exit 3
exec >> "${L}/land-${id}.log" 2>&1
branch=$(git branch --show-current)
# an early draft PR's branch holds the done mark on a commit this checkout never had
rm -f -- "${main}/.git/betterlite/${id}/ci-failures"
if git fetch -q origin "refs/heads/${branch}" 2> /dev/null && ! git merge-base --is-ancestor FETCH_HEAD HEAD; then
  git merge -q --no-edit FETCH_HEAD || {
    git merge --abort
    park "ready, but its draft PR's branch does not merge"
    exit 3
  }
fi
title=$(sed -n 's/^title: //p' "${main}/.development/backlog/${id}.yaml")
file=$(sed -n 's/^file: //p' "${main}/.development/backlog/${id}.yaml")
size=$(git diff --shortstat origin/main...HEAD -- . ':(exclude)Cargo.lock')
size=${size# }
if ! pushed=$(git push -q origin "HEAD:refs/heads/${branch}" 2>&1); then
  echo "${pushed}"
  grep -q GH013 <<< "${pushed}" && {
    say "ready, but GitHub push protection blocked the push, see logs/land-${id}.log"
    exit 5
  }
  land_again "ready, but the PR did not open: the push failed, see logs/land-${id}.log"
fi
body=${L}/pr-body-${id}.md
printf 'Builds %s, from [its task file](https://github.com/betterlite-ai/betterlite/blob/%s/.development/%s). %s, Cargo.lock aside. The lint checks pass locally and the review approved it. CI runs the tests, the oracle cases and mutants. The PR also marks %s done in the backlog.\n' "${title}" "${branch}" "${file}" "${size}" "${id}" > "${body}"
old=$(git show "origin/main:.development/${file}")
old=$(grep -o -E 'max_changed_lines: [0-9]+' <<< "${old}")
old=${old//max_changed_lines: /}
new=$(grep -o -E 'max_changed_lines: [0-9]+' ".development/${file}")
new=${new//max_changed_lines: /}
if [[ "${old}" != "${new}" ]]; then
  raised=$(git log -1 --format=%b --grep="^docs(tasks): ${id} budget " origin/main..HEAD)
  printf "\nThe architect raised its line budget from %s to %s: %s\n" "${old}" "${new}" "${raised%%$'\n'*}" >> "${body}"
fi
scoped=$(git log --reverse --format=%H --grep="^docs(tasks): ${id} scope " origin/main..HEAD)
for c in ${scoped}; do
  subject=$(git log -1 --format=%s "${c}")
  why=$(git log -1 --format=%b "${c}")
  printf "\nThe architect added %s to its scope: %s\n" "${subject#"docs(tasks): ${id} scope "}" "${why%%$'\n'*}" >> "${body}"
done
amended=$(git log --reverse --format=%H --grep="^docs(tasks): ${id} " origin/main..HEAD)
for c in ${amended}; do
  case "$(git log -1 --format=%s "${c}")" in
    "docs(tasks): ${id} budget "* | "docs(tasks): ${id} scope "*) ;;
    *)
      why=$(git log -1 --format=%b "${c}")
      printf "\nThe architect changed its task file: %s\n" "${why%%$'\n'*}" >> "${body}"
      ;;
  esac
done
pr=$(gh pr list --head "${branch}" --state open --json number -q ".[0].number")
[[ -n "${pr}" ]] || gh pr create --base main --head "${branch}" --title "${id}: ${title}" --body-file "${body}" ${PR_LABEL:+--label "${PR_LABEL}"} > /dev/null || land_again "ready, but the PR did not open, see logs/land-${id}.log"
tries=0
while [[ "${tries}" -lt 3 ]]; do
  pr=$(gh pr view "${branch}" --json number -q .number 2> /dev/null)
  [[ -n "${pr}" ]] && break
  sleep 10
  tries=$((tries + 1))
done
[[ -n "${pr}" ]] || land_again "ready, but the PR did not open, see logs/land-${id}.log"
sed -i "s/^status: .*/status: done/; s/^resolution: .*/resolution: merged/; s/^pr: .*/pr: ${pr}/" ".development/backlog/${id}.yaml"
if ! git diff --quiet -- ".development/backlog/${id}.yaml"; then
  git add ".development/backlog/${id}.yaml" && git commit -q -m "docs(backlog): ${id} done"
  if ! pushed=$(git push -q origin "HEAD:refs/heads/${branch}" 2>&1); then
    echo "${pushed}"
    land_again "PR #${pr} open, but the done-mark push failed, see logs/land-${id}.log"
  fi
fi
draft=$(gh pr view "${pr}" --json isDraft -q .isDraft)
if [[ "${draft}" = true ]]; then
  gh pr edit "${pr}" --body-file "${body}" > /dev/null
  gh pr ready "${pr}" > /dev/null || land_again "PR #${pr} is open, but it did not leave draft, see logs/land-${id}.log"
fi
# pr-watch lets go of a PR it finds unarmed, so a refused auto-merge left B5790's green PR 1431 outside the queue for 22 hours (11-12)
if ! gh pr merge "${branch}" --auto --squash > /dev/null; then
  armed=$(gh api graphql -F n="${pr}" -f query="query(\$n:Int!){repository(owner:\"betterlite-ai\",name:\"betterlite\"){pullRequest(number:\$n){autoMergeRequest{enabledAt} mergeQueueEntry{state}}}}" -q '.data.repository.pullRequest | .autoMergeRequest != null or .mergeQueueEntry != null')
  [[ "${armed}" = true ]] || land_again "PR #${pr} is open, but auto-merge did not turn on, see logs/land-${id}.log"
fi
say "PR #${pr} open: https://github.com/betterlite-ai/betterlite/pull/${pr}"
verified_head=$(git rev-parse HEAD) || exit 5
result=$(bash "${P}/pr-watch.sh" "${pr}" merge)
# a merge gets one line below, once main's run on it has finished
[[ ${result} == *"merged as"* ]] || say "${result:-pr-watch for PR #${pr} ended without a result; checkout kept}"
case "${result}" in
  *"merged as"*)
    cd "${main}" || exit 5
    # find fails on other users' unreadable /proc/*/cwd links, so only its output counts
    if ! status=$(git -C "${worktree}" status --porcelain) || [[ -n "${status}" ]] ||
      ! worktree_branch=$(git -C "${worktree}" branch --show-current) || [[ "${worktree_branch}" != "${branch}" ]] ||
      ! worktree_head=$(git -C "${worktree}" rev-parse HEAD) || [[ "${worktree_head}" != "${verified_head}" ]] ||
      {
        users=$(find /proc/[0-9]*/cwd -maxdepth 0 \( -lname "${worktree}" -o -lname "${worktree}/*" \) 2> /dev/null)
        [[ -n "${users}" ]]
      }; then
      say "merged checkout retained: changed files or a live process"
    elif git fetch --no-tags "${worktree}" "HEAD:refs/betterlite/completed/${id}" &&
      completed=$(git rev-parse "refs/betterlite/completed/${id}") && [[ "${completed}" = "${verified_head}" ]] &&
      worktree_head=$(git -C "${worktree}" rev-parse HEAD) && [[ "${worktree_head}" = "${verified_head}" ]] &&
      status=$(git -C "${worktree}" status --porcelain) && [[ -z "${status}" ]] &&
      {
        users=$(find /proc/[0-9]*/cwd -maxdepth 0 \( -lname "${worktree}" -o -lname "${worktree}/*" \) 2> /dev/null)
        [[ -z "${users}" ]]
      }; then
      if [[ -f "${worktree}/.git" ]]; then
        git worktree remove "${worktree}" || exit 5
      else
        rm -rf -- "${worktree}"
      fi
      rm -rf -- "${worktrees}/tmp/${id}" "${main}/.git/betterlite/${id}"/mutants-work*
      say "archived ${verified_head}; removed merged checkout and its temp folder"
    else
      say "merged checkout retained: archival or final idle check failed"
    fi
    # the checkout is gone before main's run finishes, so the wait holds no disk and the feeder no longer counts this flow
    result=$(bash "${P}/pr-watch.sh" "${pr}")
    say "${result:-pr-watch for PR #${pr} ended without the main run result}"
    case "${result}" in *passed) exit 0 ;; *) exit 5 ;; esac
    ;;
  # CI runs every check that builds, clippy included since stage 3, so a red run goes back to the worker with its gate lines and logs, eight rounds at most
  *" run "*" failed"*)
    run_id=
    [[ ${result} =~ run\ ([0-9]+)\ failed ]] && run_id=${BASH_REMATCH[1]}
    # in a GitHub Actions outage the check jobs never start, or all pass and the run still fails, so the flow runs CI again instead of spending a fix round
    lost=$(gh run view "${run_id}" --json jobs -q '[.jobs[] | select(.name != "gate" and (.conclusion == "failure" or .conclusion == "cancelled"))] | length' 2> /dev/null)
    # a 5xx from GitHub's API in the job that picks the parts skips every check, so it is an outage too
    if [[ "${lost}" != 0 ]]; then
      picked=$(gh run view "${run_id}" --log-failed 2> /dev/null)
      grep -q -E 'curl: \(22\) The requested URL returned error: 5[0-9]{2}' <<< "${picked}" && ! grep -q -E 'FAIL ' <<< "${picked}" && lost=0
    fi
    if [[ "${lost}" = 0 ]] && [[ "${OUTAGES:-0}" -lt 6 ]]; then
      say "CI run ${run_id} on PR #${pr} failed with no failed check, a GitHub outage; running CI again in 15 minutes"
      sleep 900
      gh run rerun "${run_id}" > /dev/null 2>&1
      OUTAGES=$((${OUTAGES:-0} + 1)) LAND_ONLY=1 exec bash "${P}/dispatch-flow.sh" "${id}" "${run}" ""
    fi
    [[ "${CI_ROUNDS:-0}" -lt 8 ]] || {
      park "CI failed on PR #${pr} (run ${run_id}) after 8 fix rounds"
      exit 5
    }
    rm -rf -- "${worktree}/target/ci-logs"
    logs=${worktree}/target/ci-logs/${run_id}
    gh run download "${run_id}" -D "${logs}" > /dev/null 2>&1
    failed_log=$(gh run view "${run_id}" --log-failed 2> /dev/null)
    failed_log=$(awk -F'\t' '$1 != "gate"' <<< "${failed_log}")
    failed_log=$(cut -f3- <<< "${failed_log}")
    failed_log=$(sed -E 's/^\xEF\xBB\xBF//; s/^[0-9TZ:.-]+ //; s/(\x1b|\^\[)\[[0-9;]*m//g' <<< "${failed_log}")
    failed_log=$(grep -v '^##\[' <<< "${failed_log}")
    lines=$(grep -m 60 -E '^FAIL |^     ' <<< "${failed_log}")
    # a step outside the gate (the gate's own tests, a tool install) prints no FAIL line, so its last lines go instead
    [[ -n "${lines}" ]] || lines=$(tail -n 40 <<< "${failed_log}")
    missed=$(find "${logs}" -path '*/mutants.out/missed.txt' -exec cat {} + 2> /dev/null)
    missed=$(sort -u <<< "${missed}")
    missed=$(head -n 40 <<< "${missed}")
    timed_out=$(find "${logs}" -path '*/mutants.out/timeout.txt' -exec cat {} + 2> /dev/null)
    timed_out=$(sort -u <<< "${timed_out}")
    timed_out=$(head -n 20 <<< "${timed_out}")
    say "CI failed on PR #${pr} (run ${run_id}); fix round $((${CI_ROUNDS:-0} + 1))"
    LAND_ONLY='' CI_ROUNDS=$((${CI_ROUNDS:-0} + 1)) exec bash "${P}/dispatch-flow.sh" "${id}" "${run}c" "CI run https://github.com/betterlite-ai/betterlite/actions/runs/${run_id} failed on PR #${pr}.
${lines}${missed:+
Mutants CI missed:
${missed}}${timed_out:+
Mutants that timed out in CI (the gate counts a timeout as a failure, so a test must fail on the mutant instead of hanging):
${timed_out}}
The gate logs of that run are in target/ci-logs/${run_id}/ in your worktree. Fix the cause and finish as usual. The final gate here builds nothing; CI runs clippy and the tests again once the branch is pushed."
    ;;
  # main moved under the open PR; the land path merges main again (line 88) and stops when the merge needs a person, three rounds at most
  *" conflicts with main"*)
    [[ "${CONFLICTS:-0}" -lt 3 ]] || {
      park "PR #${pr} conflicts with main after 3 merges of main"
      exit 5
    }
    say "PR #${pr} conflicts with main; merging main and landing again"
    LAND_ONLY=1 CONFLICTS=$((${CONFLICTS:-0} + 1)) exec bash "${P}/dispatch-flow.sh" "${id}" "${run}m" ""
    ;;
  *) exit 5 ;;
esac
