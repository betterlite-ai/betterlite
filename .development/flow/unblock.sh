#!/usr/bin/env bash
self=$(readlink -f "$0") || exit 1
P=${self%/*}
main=${BETTERLITE_ROOT:-$(git -C "${P}" rev-parse --show-toplevel)}
# Applies the rule for a blocked task on its own branch and prints the note for the next run; exits 1 when no rule applies, and the flow parks the task.
# Usage: unblock.sh <id> <worktree> <reason>
id=$1
reason=$3
cd "$2" || exit 1
task=$(git ls-files ".development/tasks/${id}-*.yaml") || exit 1
task=${task%%$'\n'*}
[[ -n "${task}" ]] || exit 1
journal=${main}/.git/betterlite/${id}
commits=$(git log --format=%s origin/main..HEAD)
indent=$(sed -n '/^  paths:$/{n;s/- .*//;p;q}' "${task}")
scope=$(sed -n '/^scope:/,/^[a-z_]*:/p' "${task}")
scope=$(sed -n "s/^${indent}- //p" <<< "${scope}")
scope=${scope//\"/}
excluded=$(sed -n '/^out_of_scope:/,/^[a-z_]*:/p' "${task}")

crate_of() {
  case "$1" in
    crates/*) cut -d/ -f1-2 <<< "$1" ;;
    tools/oracle/cases/*) cut -d/ -f1-5 <<< "$1" ;;
    tools/*) cut -d/ -f1-3 <<< "$1" ;;
    *) echo "$1" ;;
  esac
}

case "${reason}" in
  *"crate cycle"*)
    rounds=$(cat "${journal}/cycle-rounds" 2> /dev/null)
    [[ "${rounds:-0}" -lt 3 ]] || exit 1
    report=$(bash "${P}/crate-cycles.sh" "$2") && exit 1
    echo "$((${rounds:-0} + 1))" > "${journal}/cycle-rounds"
    covers=$(sed -n 's/^Scope now covers: *//p' <<< "${report}")
    read -r -a paths <<< "${covers}"
    added=
    for path in "${paths[@]}"; do
      grep -q -x -F "${path}" <<< "${scope}" && continue
      grep -q -F "${path%/\*\*}" <<< "${excluded}" && exit 1
      added="${added} ${path}"
    done
    read -r -a added_paths <<< "${added}"
    for path in "${added_paths[@]}"; do
      sed -i "0,/^  paths:\$/s||  paths:\n${indent}- \"${path}\"|" "${task}"
    done
    if [[ -n "${added}" ]]; then
      # a todo task may not cover a crate root, and the code has to leave and enter one
      backlog=.development/backlog/${id}.yaml
      sed -i 's/^status: todo$/status: in-progress/' "${backlog}"
      git add "${task}" "${backlog}"
      git commit -q -m "docs(tasks): ${id} scope${added}" -m "The branch closes a crate cycle, so a block rule let it move the code behind the loop." -- "${task}" "${backlog}" || exit 1
    fi
    echo "${report}"
    exit 0
    ;;
  *"a todo task names its own module files"*)
    # a task that creates a crate covers its root, and the crate-root check holds only for todo tasks
    crate=
    [[ ${reason} =~ scope\ covers\ crates/([A-Za-z0-9_-]+)/ ]] && crate=${BASH_REMATCH[1]}
    [[ -n "${crate}" ]] || exit 1
    git fetch -q origin 2> /dev/null
    git cat-file -e "origin/main:crates/${crate}/Cargo.toml" 2> /dev/null && exit 1
    backlog=.development/backlog/${id}.yaml
    grep -q '^status: todo$' "${backlog}" || exit 1
    sed -i 's/^status: todo$/status: in-progress/' "${backlog}"
    git add "${backlog}"
    git commit -q -m "docs(backlog): ${id} in progress" -m "The task creates crates/${crate}, so its scope covers a crate root the skeleton never declared, and the crate-root check applies only to todo tasks." -- "${backlog}" || exit 1
    echo "The task creates crates/${crate}, so a block rule set its backlog status to in-progress on this branch and task-file passes now. Keep your work and finish the task."
    exit 0
    ;;
  *"review rounds ran out"*)
    [[ -e "${journal}/auto-grant" ]] && exit 1
    touch "${journal}/auto-grant"
    echo "The review rounds ran out, and this is the one extra run the rules give. Fix every finding from the last review, then finish as usual."
    exit 0
    ;;
  *"reviewer hit its"*)
    [[ -e "${journal}/review-timeout" ]] && exit 1
    touch "${journal}/review-timeout"
    echo "The reviewer timed out before it answered, so this run asks it again. Change nothing unless a review finds something."
    exit 0
    ;;
  *RUSTSEC-*)
    # a worker can't bump a lock, and the flow merges main before the rerun
    [[ -e "${journal}/deny-rerun" ]] && exit 1
    git fetch -q origin 2> /dev/null
    base=$(git merge-base HEAD origin/main) || exit 1
    git diff --quiet "${base}" origin/main -- Cargo.lock deny.toml && exit 1
    touch "${journal}/deny-rerun"
    echo "The deny failure comes from an older main. The flow merges main before this run, and main has changed Cargo.lock or deny.toml since your base. Keep your work and finish the task."
    exit 0
    ;;
  # any other reason goes on to the budget and scope rules below
  *) ;;
esac

# a reason that only mentions the budget, with no count over it, goes on to the scope rule
need=
[[ ${reason} =~ ([0-9]+)\ (changed|counted)\ lines ]] && need=${BASH_REMATCH[1]}
# no budget line leaves old empty, and the rule below skips it
old=$(grep -o -E 'max_changed_lines: [0-9]+' "${task}")
old=${old%%$'\n'*}
old=${old#max_changed_lines: }
if [[ -n "${need}" ]] && [[ -n "${old}" ]] && [[ "${need}" -gt "${old}" ]]; then
  grep -q "^docs(tasks): ${id} budget " <<< "${commits}" && exit 1
  most=$((old * 3 / 2))
  new=$(((need * 11 / 10 / 100 + 1) * 100))
  [[ "${new}" -gt "${most}" ]] && new=${most}
  [[ "${new}" -ge "${need}" ]] || exit 1
  sed -i -E "s/max_changed_lines: [0-9]+/max_changed_lines: ${new}/" "${task}"
  git add "${task}"
  git commit -q -m "docs(tasks): ${id} budget ${new}" -m "The branch counts ${need} changed lines against a budget of ${old}, inside the 1.5x a block rule may raise it." -- "${task}" || exit 1
  echo "Your line budget is now ${new} (commit on this branch). Keep your work and finish the task."
  exit 0
fi

# dependency policy and the expect budget stay with a person; checked after the line budget, since a blocker may quote the whole budget line
grep -q -E 'deny\.toml|max_new_expects' <<< "${reason}" && exit 1

if grep -q -i -E "scope blocker|scope\.paths|widen [^.]*scope|outside ([a-z0-9']+ ){0,3}scope" <<< "${reason}"; then
  # grep -c exits 1 for a count of 0, and the count is all that matters
  scope_rounds=$(grep -c "^docs(tasks): ${id} scope " <<< "${commits}")
  [[ "${scope_rounds}" -lt 3 ]] || exit 1
  added=
  # a reason that names no path leaves nothing to add
  found=$(grep -o -E '(crates|tools)/[A-Za-z0-9_.*/-]+' <<< "${reason}") || exit 1
  found=$(sed -E 's/[.\/]+$//' <<< "${found}")
  found=$(sort -u <<< "${found}")
  # mapfile, not a for over $(...), so a path like dir/** stays a pattern instead of the files on disk
  mapfile -t paths <<< "${found}"
  for path in "${paths[@]}"; do
    grep -q -x -F "${path}" <<< "${scope}" && continue
    covered=
    while read -r glob; do
      # the scope glob as a regex, where * also crosses / as it does in [[ == ]]
      pattern=${glob//./\\.}
      pattern=${pattern//\*/.*}
      [[ ${path}/ =~ ^${pattern}$ ]] && covered=1
    done <<< "${scope}"
    [[ -n "${covered}" ]] && continue
    grep -q -F "${path}" <<< "${excluded}" && exit 1
    crate=$(crate_of "${path}")
    # a path in a crate the task doesn't touch is often only context (another task's code), so it never goes in, and the rest still can
    grep -q -E "^${crate}(/|$)" <<< "${scope}" || continue
    added="${added} ${path}"
  done
  [[ -n "${added}" ]] || exit 1
  read -r -a added_paths <<< "${added}"
  for path in "${added_paths[@]}"; do
    sed -i "0,/^  paths:\$/s||  paths:\n${indent}- \"${path}\"|" "${task}"
  done
  backlog=.development/backlog/${id}.yaml
  # a todo task may not cover a crate root
  grep -q -E '^crates/[^/]+/(src/(lib|main)\.rs|src/\*\*|\*\*)$' <<< "${added// /$'\n'}" && sed -i 's/^status: todo$/status: in-progress/' "${backlog}"
  git add "${task}" "${backlog}"
  git commit -q -m "docs(tasks): ${id} scope${added}" -m "Inside a crate this task already touches, so a block rule added it." -- "${task}" "${backlog}" || exit 1
  echo "These paths are now in your scope (commit on this branch):${added}. Keep your work and finish the task."
  exit 0
fi
exit 1
