#!/usr/bin/env bash
export GH_TOKEN="$(gh auth token --user HardMax71)"
# Wrong means: a new failed run on its branch (a failed queue run counts once the PR has left the queue, since a PR ahead in the group can cause it), a conflict with main, or leaving the queue without auto-merge.
# Usage: bash pr-watch.sh <pr>
pr=$1
cd "${BETTERLITE_ROOT:-/home/user/Desktop/betterlite}" || exit 1
query='query($n:Int!){repository(owner:"betterlite-ai",name:"betterlite"){pullRequest(number:$n){state headRefName mergeStateStatus autoMergeRequest{enabledAt} mergeQueueEntry{state} mergeCommit{oid}}}}'
failed_runs() {
  gh run list --workflow gate.yml --limit 30 --json databaseId,headBranch,conclusion \
    -q ".[] | select(.conclusion == \"failure\") | select(.headBranch == \"$1\") | .databaseId" 2>/dev/null
}
last_queue_failure() {
  gh run list --workflow gate.yml --limit 30 --json databaseId,headBranch,conclusion -q "[.[] | select(.conclusion == \"failure\") | select(.headBranch | startswith(\"gh-readonly-queue/main/pr-$pr-\"))][0].databaseId // empty" 2>/dev/null
}
first_fail() {
  gh run view "$1" --log-failed 2>/dev/null | grep -m2 -o -E 'FAIL [a-z-]+ \([0-9]+s\): .*' | tr '\n' ' '
}
branch=$(gh pr view "$pr" --json headRefName -q .headRefName) || exit 1
known=$(failed_runs "$branch")
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
  if [ "$merge_state" = DIRTY ]; then echo "PR $pr conflicts with main"; exit 1; fi
  if [ "$auto" = false ] && [ "$queued" = false ]; then outside=$((outside + 1)); else outside=0; fi
  # GitHub sometimes leaves a clean PR with auto-merge outside the queue; re-arming queues it.
  if [ "$auto" = true ] && [ "$queued" = false ] && [ "$merge_state" = CLEAN ]; then stuck=$((stuck + 1)); else stuck=0; fi
  if [ "$stuck" -ge 5 ]; then gh pr merge "$pr" --disable-auto > /dev/null 2>&1; gh pr merge "$pr" --auto --squash > /dev/null 2>&1; stuck=0; fi
  if [ "$outside" -ge 3 ]; then queue_run=$(last_queue_failure); echo "PR $pr is neither queued nor set to auto-merge ($merge_state)${queue_run:+; last queue run $queue_run failed: $(first_fail "$queue_run")}"; exit 1; fi
  sleep 60
done
