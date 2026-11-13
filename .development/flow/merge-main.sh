#!/usr/bin/env bash
self=$(readlink -f "$0") || exit 1
P=${self%/*}
main=${BETTERLITE_ROOT:-$(git -C "${P}" rev-parse --show-toplevel)}
worktrees=${BETTERLITE_WORKTREES:-$(dirname "${main}")/betterlite-worktrees}
# Merges origin/main into a task worktree and resolves the known list conflicts: .development files (main wins, workers never edit them; the task's own task file keeps the higher of the two line budgets, since the architect raises a budget in the branch), manifest dependency lines (one line per name, main's wins), Cargo.lock, config structs. A helper registry conflict stops it, since the registry is pre-seeded and a task never edits it.
# Usage: bash merge-main.sh <worktree>. Leaves the merge uncommitted when a conflict is outside those files, and stops when the merged manifests do not load.
cd "$1" || exit 1
id=$(basename "$1")
git fetch -q origin 2> /dev/null
# a Codex worker can leave an empty HEAD.lock seconds after its last commit, and git fills a live one within milliseconds
for name in HEAD.lock index.lock; do
  lock=$(git rev-parse --git-dir)/${name}
  if [[ -f "${lock}" ]] && [[ ! -s "${lock}" ]] && stale=$(find "${lock}" -mmin +1) && [[ -n "${stale}" ]]; then
    made=$(date -r "${lock}" +%H:%M:%S)
    rm -f -- "${lock}" && printf '%(%H:%M)T %s\n' -1 "${id} removed a stale empty ${name} from ${made}" >> "${worktrees}/logs/flow-events.log"
  fi
done
lock_and_check() {
  if ! problem=$(cargo metadata --offline --format-version 1 2>&1 > /dev/null) && ! cargo metadata --format-version 1 > /dev/null 2>&1; then
    # a crate loop goes back to the worker through crate-cycles.sh
    grep -q "cyclic package dependency" <<< "${problem}" && return
    echo "cargo metadata failed after the merge"
    exit 1
  fi
}
if git merge --no-edit origin/main > /dev/null; then
  lock_and_check
  if ! git diff --quiet -- Cargo.lock; then git add Cargo.lock && git commit -q -m "chore(deps): refresh Cargo.lock after merging main"; fi
  echo "merged cleanly"
  exit 0
fi
conflicts=$(git diff --name-only --diff-filter=U)
# grep and git add below take one argument per conflicted file
read -r -d '' -a conflicted <<< "${conflicts}"
for file in ${conflicts}; do
  case "${file}" in
    Cargo.lock) git checkout -q --theirs Cargo.lock ;;
    .development/tasks/"${id}"-*)
      echo "task amendment conflicts with main: ${file}; architect resolution required"
      exit 1
      ;;
    .development/*) git checkout -q --theirs "${file}" ;;
    crates/betterlite-config/src/raw.rs | crates/betterlite-config/src/types.rs | crates/betterlite-config/src/loader.rs | crates/betterlite-config/tests/config_tests/snapshots/*.snap) perl -0pi -e 's/^<<<<<<< HEAD\n(.*?)^=======\n(.*?)^>>>>>>> origin\/main\n/$1$2/msg' "${file}" ;;
    */Cargo.toml | Cargo.toml) perl "${P}/resolve-manifest.pl" "${file}" ;;
    *) echo "unhandled conflict: ${file}" ;;
  esac
done
left=$(grep -l -E '^(<<<<<<<|>>>>>>>) ' "${conflicted[@]}" 2> /dev/null)
if [[ -n "${left}" ]]; then
  echo "markers left in: ${left}"
  exit 1
fi
if echo "${conflicts}" | grep -q -v -E '^(Cargo.lock|\.development/.*|(.*/)?Cargo.toml|crates/betterlite-config/src/(raw|types|loader).rs|crates/betterlite-config/tests/config_tests/snapshots/.*\.snap)$'; then
  echo "merge left open for the unhandled files"
  exit 1
fi
lock_and_check
cargo fmt -p betterlite-oracle > /dev/null 2>&1
git add "${conflicted[@]}" Cargo.lock && git commit -q --no-edit && echo "merged with resolved: ${conflicted[*]} "
