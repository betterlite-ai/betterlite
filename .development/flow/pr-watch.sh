#!/usr/bin/env bash
export GH_TOKEN="$(gh auth token --user HardMax71)"
export GH_REPO=betterlite-ai/betterlite
# Wrong means: a new failed run on its branch (a failed queue run counts once the PR has left the queue, since a PR ahead in the group can cause it), a conflict with main, or leaving the queue without auto-merge.
# Usage: bash pr-watch.sh <pr>
pr=$1
P=$(dirname "$(readlink -f "$0")")
cd "${BETTERLITE_ROOT:-$(git -C "$P" rev-parse --show-toplevel)}" || exit 1
query='query($n:Int!){repository(owner:"betterlite-ai",name:"betterlite"){pullRequest(number:$n){state headRefName mergeStateStatus autoMergeRequest{enabledAt} mergeQueueEntry{state} mergeCommit{oid}}}}'
failed_runs() {
  gh run list --workflow gate.yml --branch "$1" --limit 30 --json databaseId,conclusion \
    -q '.[] | select(.conclusion == "failure") | .databaseId' 2>/dev/null
}
last_queue_failure() {
  gh run list --workflow gate.yml --limit 30 --json databaseId,headBranch,conclusion -q "[.[] | select(.conclusion == \"failure\") | select(.headBranch | startswith(\"gh-readonly-queue/main/pr-$pr-\"))][0].databaseId // empty" 2>/dev/null
}
first_fail() {
  gh run view "$1" --log-failed 2>/dev/null | grep -a -o -E 'FAIL [a-z-]+ \([0-9]+s\): .*|FAIL \[ *[0-9.]+s\] (\([0-9]+/[0-9]+\) )?[A-Za-z0-9_-]+ [^ ]+' | sed -E 's#\([0-9]+/[0-9]+\) ##' | awk '!seen[$0]++' | head -2 | tr '\n' ' '
}
until branch=$(gh pr view "$pr" --json headRefName -q .headRefName 2>/dev/null); do sleep 60; done
known=$(failed_runs "$branch"; gh run list --workflow gate.yml --branch "$branch" --limit 1 --json databaseId,conclusion -q '.[] | select(.conclusion == "cancelled") | .databaseId' 2>/dev/null)
outside=0
stuck=0
while true; do
  info=$(gh api graphql -F n="$pr" -f query="$query" -q '.data.repository.pullRequest | "\(.state) \(.mergeStateStatus) \(.autoMergeRequest != null) \(.mergeQueueEntry != null) \(.mergeCommit.oid // "-")"' 2>/dev/null)
  if [ -z "$info" ]; then sleep 60; continue; fi
  read -r state merge_state auto queued commit <<< "$info"
  if [ "$state" = CLOSED ]; then echo "PR $pr closed without merging"; exit 1; fi
  if [ "$state" = MERGED ]; then
    for i in $(seq 1 60); do
      git fetch -q origin main 2>/dev/null
      run=""
      for line in $(gh run list --workflow gate.yml --branch main --event push --limit 10 --json databaseId,headSha,status,conclusion -q 'reverse | .[] | select(.conclusion != "cancelled") | "\(.databaseId),\(.headSha),\(.status),\(.conclusion)"' 2>/dev/null); do
        if git merge-base --is-ancestor "$commit" "$(cut -d, -f2 <<< "$line")" 2>/dev/null; then run=$line; break; fi
      done
      IFS=, read -r run_id _ run_status run_conclusion <<< "$run"
      if [ "$run_status" = completed ]; then
        if [ "$run_conclusion" = success ]; then echo "PR $pr merged as ${commit:0:8}; main run $run_id passed"; exit 0; fi
        echo "PR $pr merged as ${commit:0:8}; main run $run_id $run_conclusion: $(first_fail "$run_id")"; exit 1
      fi
      sleep 60
    done
    echo "PR $pr merged as ${commit:0:8}; main run did not finish within an hour"; exit 1
  fi
  for run_id in $(failed_runs "$branch"); do
    if ! grep -qx "$run_id" <<< "$known"; then echo "PR $pr run $run_id failed: $(first_fail "$run_id")"; exit 1; fi
  done
  # a job that hits its timeout ends the run as cancelled, not failed; a newer run on the branch means a push cancelled it instead
  newest=$(gh run list --workflow gate.yml --branch "$branch" --limit 1 --json databaseId,conclusion -q '.[] | select(.conclusion == "cancelled") | .databaseId' 2>/dev/null)
  if [ -n "$newest" ] && ! grep -qx "$newest" <<< "$known"; then echo "PR $pr run $newest failed: cancelled, a job hit its time limit (a hung test or check)"; exit 1; fi
  if [ "$merge_state" = DIRTY ]; then echo "PR $pr conflicts with main"; exit 1; fi
  if [ "$auto" = false ] && [ "$queued" = false ]; then outside=$((outside + 1)); else outside=0; fi
  # GitHub sometimes leaves a clean PR with auto-merge outside the queue; re-arming queues it.
  if [ "$auto" = true ] && [ "$queued" = false ] && [ "$merge_state" = CLEAN ]; then stuck=$((stuck + 1)); else stuck=0; fi
  if [ "$stuck" -ge 5 ]; then gh pr merge "$pr" --disable-auto > /dev/null 2>&1; gh pr merge "$pr" --auto --squash > /dev/null 2>&1; stuck=0; fi
  if [ "$outside" -ge 3 ]; then queue_run=$(last_queue_failure); echo "PR $pr is neither queued nor set to auto-merge ($merge_state)${queue_run:+; last queue run $queue_run failed: $(first_fail "$queue_run")}"; exit 1; fi
  sleep 60
done
