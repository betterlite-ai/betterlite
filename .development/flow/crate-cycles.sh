#!/usr/bin/env bash
P=$(dirname "$(readlink -f "$0")")
main=${BETTERLITE_ROOT:-$(git -C "$P" rev-parse --show-toplevel)}
worktrees=${BETTERLITE_WORKTREES:-$(dirname "$main")/betterlite-worktrees}
# Prints the crate loop this branch closes with main and the other running branches, what needs each edge and where the code can go, and exits 1; prints nothing and exits 0 when there is none.
# Usage: crate-cycles.sh <worktree>. cargo metadata --no-deps still reads manifests that form a loop, and tsort finds it. Dev-dependencies don't count, since cargo allows loops through them.
edges() {
  cargo metadata --no-deps --format-version 1 --manifest-path "$1/Cargo.toml" 2> /dev/null |
    jq -r '.workspace_root as $root | .packages[] as $p | $p.dependencies[] | select(.path != null and (.kind == null or .kind == "build")) | "\($p.name) \(.name) \($p.manifest_path | ltrimstr($root + "/") | rtrimstr("/Cargo.toml"))"' |
    sort -u
}

# only manifests the checkout changed since its own merge base count, so a branch behind main doesn't report an edge main has dropped
added() {
  base=$(git -C "$1" merge-base HEAD origin/main) || return 0
  changed=$(git -C "$1" diff --name-only "$base" -- '*Cargo.toml')
  [ -n "$changed" ] || return 0
  edges "$1" | while read -r from to dir; do
    grep -q -x -F "$dir/Cargo.toml" <<< "$changed" || continue
    git -C "$1" show "$base:$dir/Cargo.toml" 2> /dev/null | sed '/dev-dependencies\]/,/^\[/d' | grep -q -E "^$to( |\.|=)" || echo "$from $to $dir $2"
  done
}

cd "$1" || exit 1
id=$(basename "$PWD")
mine=$(added "$PWD" "this-branch")
[ -n "$mine" ] || exit 0
tree=$(edges "$PWD")
others=$(for args in "$worktrees"/logs/flows/*.args; do
  other=$(basename "$args" .args)
  [ "$other" = "$id" ] || [ ! -d "$worktrees/$other" ] || added "$worktrees/$other" "$other"
done)
all=$(printf '%s\n' "$tree" "$others" | cut -d' ' -f1,2 | sort -u)

loops=$(LC_ALL=C tsort <<< "$all" 2>&1 > /dev/null | awk '/contains a loop/ { if (l) print l; l = ""; next } { l = l " " $2 } END { if (l) print l }')
loop=
while read -r members; do
  while read -r from to _; do
    [[ " $members " == *" $from "* && " $members " == *" $to "* ]] && loop=$members
  done <<< "$mine"
  [ -n "$loop" ] && break
done <<< "$loops"
[ -n "$loop" ] || exit 0

echo "crate cycle between: $loop. Cargo refuses a dependency loop, so this branch can't land with it."
scope=
for crate in $loop; do
  [ -d "crates/$crate" ] && scope="$scope crates/$crate/**"
done
labelled=$(printf '%s\n' "$mine" "$others"; grep -v -x -F -f <(cut -d' ' -f1-3 <<< "$mine") <<< "$tree" | sed 's/$/ main/')
while read -r from to dir owner; do
  [[ " $loop " == *" $from "* && " $loop " == *" $to "* ]] || continue
  checkout=$PWD
  case "$owner" in main|this-branch) ;; *) checkout=$worktrees/$owner ;; esac
  users=$(git -C "$checkout" grep --untracked -l -w "${to//-/_}" -- "$dir/src")
  echo "$from > $to ($owner) is used by: $(paste -sd' ' - <<< "$users")"
  modules=$(sed -E "s|^$dir/src/([^/.]+).*|\1|" <<< "$users" | sort -u)
  case "$modules" in ''|*$'\n'*|lib|main) continue ;; esac
  without=$(grep -v -x -F "$from $to" <<< "$all")
  takers=
  for folder in crates/*/; do
    crate=$(basename "$folder")
    [[ " $loop " == *" $crate "* ]] && continue
    named=
    for word in ${modules//_/ }; do
      [[ "-$crate-" == *"-$word-"* ]] && named=1
    done
    [ -n "$named" ] || git grep -q -w "${from//-/_}::$modules" -- "$folder" || continue
    LC_ALL=C tsort <<< "$without"$'\n'"$crate $to" > /dev/null 2>&1 && takers="$takers $crate" && scope="$scope $folder**"
  done
  [ -z "$takers" ] || echo "The module $modules is the only reason for that edge. Crates that can take it without a loop:$takers"
  for file in $(git grep -l -w "${from//-/_}::$modules" -- crates tools); do
    scope="$scope $file"
  done
done <<< "$labelled"
shared=$(for crate in $loop; do grep "^$crate " <<< "$all" | cut -d' ' -f2 | sort -u; done | sort | uniq -c | awk -v n="$(wc -w <<< "$loop")" '$1 == n { print $2 }' | paste -sd' ' -)
if [ -n "$shared" ]; then
  echo "Every crate in the loop already depends on: $shared"
  for crate in $shared; do
    [ -d "crates/$crate" ] && scope="$scope crates/$crate/**"
  done
fi
echo "Break the loop by moving code. Don't copy it and don't drop behaviour. If what one side needs uses nothing above a crate the whole loop already depends on, move it into that crate. If one module is the only reason for an edge, move it to a crate listed for it and fix its imports. If neither works, report blocked and say why."
echo "Scope now covers:$(tr ' ' '\n' <<< "$scope" | sort -u | paste -sd' ' -)"
exit 1
