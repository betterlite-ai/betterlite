#!/usr/bin/env bash
# Opens a task's PR as a draft when its first review starts, so CI runs the tests, oracle cases and mutants while the review runs (Max, 11-12). Each review start pushes the reviewed head with the done mark on top, as a commit the checkout never sees, and a failed CI run on it goes to the journal's ci-failures file for the gate's next fix turn. A non-draft PR belongs to the land, so this leaves it alone. Usage: bash early-pr.sh <id> <flow pid>
self=$(readlink -f "$0") || exit 1
P=${self%/*}
main=${BETTERLITE_ROOT:-$(git -C "${P}" rev-parse --show-toplevel)}
worktrees=${BETTERLITE_WORKTREES:-$(dirname "${main}")/betterlite-worktrees}
GH_TOKEN=$(gh auth token --user HardMax71)
export GH_TOKEN
export GH_REPO=betterlite-ai/betterlite
id=$1
flow=$2
journal=${main}/.git/betterlite/${id}
worktree=${worktrees}/${id}
started=$(date +%s)
reviewed=
pushed=
pr=
while kill -0 "${flow}" 2> /dev/null; do
  latest=0
  for file in "${journal}"/attempt-*-reviewer.started.json; do
    [[ -e "${file}" ]] || continue
    stamp=$(stat -c %Y "${file}")
    [[ "${stamp}" -gt "${latest}" ]] && latest=${stamp}
  done
  if [[ "${latest}" -ge "${started}" ]] && head=$(git -C "${worktree}" rev-parse HEAD) && [[ "${head}" != "${reviewed}" ]]; then
    reviewed=${head}
    branch=$(git -C "${worktree}" branch --show-current) || exit 1
    title=$(sed -n 's/^title: //p' "${main}/.development/backlog/${id}.yaml")
    if [[ -z "${pr}" ]]; then
      open=$(gh pr list --head "${branch}" --state open --json number,isDraft -q '.[] | "\(.number) \(.isDraft)"') || exit 1
      read -r pr draft <<< "${open}"
      [[ -n "${pr}" ]] && [[ "${draft}" != true ]] && exit 0
    fi
    if [[ -z "${pr}" ]]; then
      git -C "${worktree}" push -q origin "${head}:refs/heads/${branch}" || exit 1
      gh pr create --draft --base main --head "${branch}" --title "${id}: ${title}" --body "${id} passed its local gate and its review is running. CI runs on this draft now, and the flow marks it ready with auto-merge once the review approves." > /dev/null || exit 1
      pr=$(gh pr view "${branch}" --json number -q .number) || exit 1
    fi
    remote=$(git -C "${worktree}" ls-remote origin "refs/heads/${branch}") || exit 1
    remote=${remote%%$'\t'*}
    [[ -n "${remote}" ]] && git -C "${worktree}" fetch -q origin "refs/heads/${branch}"
    parents=(-p "${head}")
    [[ -n "${remote}" ]] && ! git -C "${worktree}" merge-base --is-ancestor "${remote}" "${head}" && parents+=(-p "${remote}")
    yaml=$(git -C "${worktree}" show "${head}:.development/backlog/${id}.yaml") || exit 1
    marked=$(sed "s/^status: .*/status: done/; s/^resolution: .*/resolution: merged/; s/^pr: .*/pr: ${pr}/" <<< "${yaml}")
    blob=$(git -C "${worktree}" hash-object -w --stdin <<< "${marked}") || exit 1
    index=$(mktemp "${worktrees}/tmp/${id}/early-pr-index.XXXXXX") || exit 1
    GIT_INDEX_FILE=${index} git -C "${worktree}" read-tree "${head}" || exit 1
    GIT_INDEX_FILE=${index} git -C "${worktree}" update-index --cacheinfo "100644,${blob},.development/backlog/${id}.yaml" || exit 1
    tree=$(GIT_INDEX_FILE=${index} git -C "${worktree}" write-tree) || exit 1
    rm -f -- "${index}"
    pushed=$(git -C "${worktree}" commit-tree "${tree}" "${parents[@]}" -m "docs(backlog): ${id} done") || exit 1
    git -C "${worktree}" push -q origin "${pushed}:refs/heads/${branch}" || exit 1
    printf '%(%H:%M)T %s\n' -1 "${id} draft PR #${pr} has the reviewed head ${head:0:8}: https://github.com/${GH_REPO}/pull/${pr}" >> "${worktrees}/logs/flow-events.log"
  fi
  if [[ -n "${pushed}" ]]; then
    # one REST call per poll, as in pr-watch.sh
    run=$(gh api "repos/${GH_REPO}/actions/workflows/gate.yml/runs?branch=${branch}&per_page=10" -q ".workflow_runs[] | select(.head_sha == \"${pushed}\") | \"\(.id) \(.status) \(.conclusion)\"" 2> /dev/null)
    read -r run_id status conclusion <<< "${run}"
    if [[ "${status}" = completed ]]; then
      if [[ "${conclusion}" = failure ]]; then
        fail=$(bash "${P}/first-fail.sh" "${run_id}")
        printf '%s\nCI run https://github.com/%s/actions/runs/%s failed on draft PR #%s: %s\n' "${reviewed}" "${GH_REPO}" "${run_id}" "${pr}" "${fail:-see the run log}" > "${journal}/ci-failures"
      fi
      pushed=
    fi
  fi
  sleep 120
done
