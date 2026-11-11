#!/usr/bin/env bash
export GH_TOKEN="$(gh auth token --user HardMax71)"
export GH_REPO=betterlite-ai/betterlite
# Wrong means: a new failed run on its branch (a failed queue run counts once the PR has left the queue, since a PR ahead in the group can cause it), a conflict with main, or leaving the queue without auto-merge.
# Usage: bash pr-watch.sh <pr>
pr=$1
# the land enables auto-merge seconds before this starts, and the queue can drop the PR within that time
since=$(date -u -d "-2 min" +%Y-%m-%dT%H:%M:%SZ)
P=$(dirname "$(readlink -f "$0")")
cd "${BETTERLITE_ROOT:-$(git -C "$P" rev-parse --show-toplevel)}" || exit 1
query='query($n:Int!){repository(owner:"betterlite-ai",name:"betterlite"){pullRequest(number:$n){state headRefName mergeStateStatus autoMergeRequest{enabledAt} mergeQueueEntry{state} mergeCommit{oid}}}}'
# One REST call per poll: gh run list --workflow spends a second call resolving the workflow, and 28 watchers listing twice a minute used up the account's 5000 calls an hour
branch_runs() {
  gh api "repos/$GH_REPO/actions/workflows/gate.yml/runs?branch=$1&per_page=30" \
    -q '.workflow_runs[] | "\(.id) \(.run_attempt) \(.conclusion)"' 2>/dev/null
}
last_queue_failure() {
  gh run list --workflow gate.yml --limit 30 --json databaseId,headBranch,conclusion -q "[.[] | select(.conclusion == \"failure\") | select(.headBranch | startswith(\"gh-readonly-queue/main/pr-$pr-\"))][0].databaseId // empty" 2>/dev/null
}
queue_removal() {
  gh api graphql -F n="$pr" -f query='query($n:Int!){repository(owner:"betterlite-ai",name:"betterlite"){pullRequest(number:$n){timelineItems(last:1,itemTypes:[REMOVED_FROM_MERGE_QUEUE_EVENT]){nodes{... on RemovedFromMergeQueueEvent{createdAt reason}}}}}}' \
    -q ".data.repository.pullRequest.timelineItems.nodes[] | select(.createdAt >= \"$since\") | .reason" 2>/dev/null
}
first_fail() {
  gh run view "$1" --log-failed 2>/dev/null | grep -a -o -E 'FAIL [a-z-]+ \([0-9]+s\): .*|FAIL \[ *[0-9.]+s\] (\([0-9]+/[0-9]+\) )?[A-Za-z0-9_-]+ [^ ]+' | sed -E 's#\([0-9]+/[0-9]+\) ##' | awk '!seen[$0]++' | head -2 | tr '\n' ' '
}
until branch=$(gh pr view "$pr" --json headRefName -q .headRefName 2>/dev/null); do sleep 60; done
# a refused list (rate limit, outage) would leave the old failed runs looking new, and each would cost a fix round
until runs=$(branch_runs "$branch"); do sleep 60; done
known=$(awk '$3 == "failure" {print $1 ":" $2} NR == 1 && $3 == "cancelled" {print $1}' <<< "$runs")
outside=0
stuck=0
rearmed=0
while true; do
  info=$(gh api graphql -F n="$pr" -f query="$query" -q '.data.repository.pullRequest | "\(.state) \(.mergeStateStatus) \(.autoMergeRequest != null) \(.mergeQueueEntry != null) \(.mergeCommit.oid // "-")"' 2>/dev/null)
  if [ -z "$info" ]; then sleep 60; continue; fi
  read -r state merge_state auto queued commit <<< "$info"
  if [ "$state" = CLOSED ]; then echo "PR $pr closed without merging"; exit 1; fi
  if [ "$state" = MERGED ]; then
    for i in $(seq 1 60); do
      git fetch -q origin main 2>/dev/null
      run=""
      for line in $(gh api "repos/$GH_REPO/actions/workflows/gate.yml/runs?branch=main&event=push&per_page=10" -q '.workflow_runs | reverse | .[] | select(.conclusion != "cancelled") | "\(.id),\(.head_sha),\(.status),\(.conclusion)"' 2>/dev/null); do
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
  runs=$(branch_runs "$branch")
  # a run that failed again after a rerun is new, so a run counts by its attempt too
  for attempt in $(awk '$3 == "failure" {print $1 ":" $2}' <<< "$runs"); do
    if ! grep -qx "$attempt" <<< "$known"; then echo "PR $pr run ${attempt%:*} failed: $(first_fail "${attempt%:*}")"; exit 1; fi
  done
  # a job that hits its timeout ends the run as cancelled, not failed; a newer run on the branch means a push cancelled it instead
  newest=$(awk 'NR == 1 && $3 == "cancelled" {print $1}' <<< "$runs")
  if [ -n "$newest" ] && ! grep -qx "$newest" <<< "$known"; then echo "PR $pr run $newest failed: cancelled, a job hit its time limit (a hung test or check)"; exit 1; fi
  if [ "$merge_state" = DIRTY ]; then echo "PR $pr conflicts with main"; exit 1; fi
  if [ "$auto" = false ] && [ "$queued" = false ]; then outside=$((outside + 1)); else outside=0; fi
  # GitHub sometimes leaves a clean PR with auto-merge outside the queue; re-arming queues it.
  if [ "$auto" = true ] && [ "$queued" = false ] && [ "$merge_state" = CLEAN ]; then stuck=$((stuck + 1)); else stuck=0; fi
  if [ "$stuck" -ge 5 ]; then gh pr merge "$pr" --disable-auto > /dev/null 2>&1; gh pr merge "$pr" --auto --squash > /dev/null 2>&1; stuck=0; fi
  if [ "$outside" -ge 3 ]; then
    reason=$(queue_removal) || { sleep 60; continue; }
    case "$reason" in
      failed_checks) queue_run=$(last_queue_failure); echo "PR $pr is neither queued nor set to auto-merge ($merge_state)${queue_run:+; last queue run $queue_run failed: $(first_fail "$queue_run")}"; exit 1 ;;
      merge_conflict) echo "PR $pr conflicts with main (the merge queue removed it for merge_conflict)"; exit 1 ;;
      # no failed check is behind these, and reporting an older failed queue run would cost a fix round
      invalid_merge_commit | checks_timed_out)
        if [ "$rearmed" -lt 3 ]; then gh pr merge "$pr" --auto --squash > /dev/null 2>&1; rearmed=$((rearmed + 1)); outside=0; sleep 60; continue; fi ;;
      # off with no removal means turned off by hand or never set by the land, so it lets go and names no run
      "") echo "PR $pr is neither queued nor set to auto-merge ($merge_state); the merge queue has not removed it since the land"; exit 1 ;;
    esac
    echo "PR $pr is neither queued nor set to auto-merge ($merge_state); the merge queue removed it for $reason, auto-merge re-armed $rearmed times"; exit 1
  fi
  sleep 60
done
