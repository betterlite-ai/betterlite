#!/usr/bin/env bash
P=$(dirname "$(readlink -f "$0")")
main=${BETTERLITE_ROOT:-$(git -C "$P" rev-parse --show-toplevel)}
# Applies the rule for a blocked task on its own branch and prints the note for the next run; exits 1 when no rule applies, and the flow parks the task.
# Usage: unblock.sh <id> <worktree> <reason>
id=$1
reason=$3
cd "$2" || exit 1
task=$(git ls-files ".development/tasks/$id-*.yaml" | head -n 1)
[ -n "$task" ] || exit 1
journal=$main/.git/betterlite/$id
commits=$(git log --format=%s origin/main..HEAD)

crate_of() {
  case "$1" in
    crates/*) cut -d/ -f1-2 <<< "$1" ;;
    tools/oracle/cases/*) cut -d/ -f1-5 <<< "$1" ;;
    tools/*) cut -d/ -f1-3 <<< "$1" ;;
    *) echo "$1" ;;
  esac
}

case "$reason" in
  *"review rounds ran out"*)
    [ -e "$journal/auto-grant" ] && exit 1
    touch "$journal/auto-grant"
    echo "The review rounds ran out, and this is the one extra run the rules give. Fix every finding from the last review, then finish as usual."
    exit 0 ;;
esac

# dependency policy and task-format changes stay with a person
grep -q -E 'deny\.toml|max_new_expects|\.development/' <<< "$reason" && exit 1

if grep -q -E 'FAIL diff-budget|max_changed_lines|[0-9]+ (changed|counted) lines' <<< "$reason"; then
  grep -q "^docs(tasks): $id budget " <<< "$commits" && exit 1
  need=$(grep -o -E '[0-9]+ (changed|counted) lines' <<< "$reason" | head -n 1 | cut -d' ' -f1)
  [ -n "$need" ] || exit 1
  old=$(sed -n 's/^  max_changed_lines: //p' "$task")
  [ -n "$old" ] || exit 1
  most=$((old * 3 / 2))
  new=$(( (need * 11 / 10 / 100 + 1) * 100 ))
  [ "$new" -gt "$most" ] && new=$most
  [ "$new" -ge "$need" ] && [ "$new" -gt "$old" ] || exit 1
  sed -i "s/^  max_changed_lines: .*/  max_changed_lines: $new/" "$task"
  git add "$task"
  git commit -q -m "docs(tasks): $id budget $new" -m "The branch counts $need changed lines against a budget of $old, inside the 1.5x a block rule may raise it." -- "$task" || exit 1
  echo "Your line budget is now $new (commit on this branch). Keep your work and finish the task."
  exit 0
fi

if grep -q -i -E "scope blocker|scope\.paths|widen [^.]*scope|outside ([a-z0-9']+ ){0,3}scope" <<< "$reason"; then
  [ "$(grep -c "^docs(tasks): $id scope " <<< "$commits")" -lt 3 ] || exit 1
  indent=$(sed -n '/^  paths:$/{n;s/- .*//;p;q}' "$task")
  scope=$(sed -n '/^scope:/,/^[a-z_]*:/p' "$task" | sed -n "s/^$indent- //p" | tr -d '"')
  excluded=$(sed -n '/^out_of_scope:/,/^[a-z_]*:/p' "$task")
  added=
  for path in $(grep -o -E '(crates|tools)/[A-Za-z0-9_.*/-]+' <<< "$reason" | sed -E 's/[.\/]+$//' | sort -u); do
    grep -q -x -F "$path" <<< "$scope" && continue
    grep -q -F "$path" <<< "$excluded" && exit 1
    crate=$(crate_of "$path")
    grep -q -E "^$crate(/|$)" <<< "$scope" || exit 1
    added="$added $path"
  done
  [ -n "$added" ] || exit 1
  for path in $added; do
    sed -i "0,/^  paths:\$/s||  paths:\n$indent- \"$path\"|" "$task"
  done
  git add "$task"
  git commit -q -m "docs(tasks): $id scope$added" -m "Inside a crate this task already touches, so a block rule added it." -- "$task" || exit 1
  echo "These paths are now in your scope (commit on this branch):$added. Keep your work and finish the task."
  exit 0
fi
exit 1
