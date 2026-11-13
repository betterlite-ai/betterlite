#!/usr/bin/env bash
self=$(readlink -f "$0") || exit 1
P=${self%/*}
main=${BETTERLITE_ROOT:-$(git -C "${P}" rev-parse --show-toplevel)}
worktrees=${BETTERLITE_WORKTREES:-$(dirname "${main}")/betterlite-worktrees}
# Prints the crate loop this branch closes with main and the other running branches, what needs each edge and where the code can go, and exits 1; prints nothing and exits 0 when there is none.
# Usage: crate-cycles.sh <worktree>. cargo metadata --no-deps still reads manifests that form a loop, and tsort finds it. Dev-dependencies don't count, since cargo allows loops through them.
# manifests cargo can't read give no edges, so they never send a task down the crate cycle path
edges() {
  metadata=$(cargo metadata --no-deps --format-version 1 --manifest-path "$1/Cargo.toml" 2> /dev/null) || return 0
  pairs=$(jq -r '.workspace_root as $root | .packages[] as $p | $p.dependencies[] | select(.path != null and (.kind == null or .kind == "build")) | "\($p.name) \(.name) \($p.manifest_path | ltrimstr($root + "/") | rtrimstr("/Cargo.toml"))"' <<< "${metadata}")
  [[ -z "${pairs}" ]] || sort -u <<< "${pairs}"
}

# only manifests the checkout changed since its own merge base count, so a branch behind main doesn't report an edge main has dropped
added() {
  base=$(git -C "$1" merge-base HEAD origin/main) || return 0
  changed=$(git -C "$1" diff --name-only "${base}" -- '*Cargo.toml')
  [[ -n "${changed}" ]] || return 0
  list=$(edges "$1")
  [[ -n "${list}" ]] || return 0
  while read -r from to dir; do
    grep -q -x -F "${dir}/Cargo.toml" <<< "${changed}" || continue
    # a manifest missing at the base is new on this branch, so every edge in it counts as added
    manifest=$(git -C "$1" show "${base}:${dir}/Cargo.toml" 2> /dev/null)
    manifest=$(sed '/dev-dependencies\]/,/^\[/d' <<< "${manifest}")
    grep -q -E "^${to}( |\.|=)" <<< "${manifest}" || echo "${from} ${to} ${dir} $2"
  done <<< "${list}"
}

cd "$1" || exit 1
id=$(basename "${PWD}")
mine=$(added "${PWD}" "this-branch")
[[ -n "${mine}" ]] || exit 0
tree=$(edges "${PWD}")
others=$(for args in "${worktrees}"/logs/flows/*.args; do
  other=$(basename "${args}" .args)
  [[ "${other}" = "${id}" ]] || [[ ! -d "${worktrees}/${other}" ]] || added "${worktrees}/${other}" "${other}"
done)
all=$(cut -d' ' -f1,2 <<< "${tree}"$'\n'"${others}")
all=$(sort -u <<< "${all}")

# tsort exits 1 when it finds a loop; the loops are in what it prints
cycles=$(LC_ALL=C tsort <<< "${all}" 2>&1 > /dev/null)
[[ -n "${cycles}" ]] || exit 0
loops=$(awk '/contains a loop/ { if (l) print l; l = ""; next } { l = l " " $2 } END { if (l) print l }' <<< "${cycles}")
loop=
while read -r members; do
  while read -r from to _; do
    [[ " ${members} " == *" ${from} "* && " ${members} " == *" ${to} "* ]] && loop=${members}
  done <<< "${mine}"
  [[ -n "${loop}" ]] && break
done <<< "${loops}"
[[ -n "${loop}" ]] || exit 0

echo "crate cycle between: ${loop}. Cargo refuses a dependency loop, so this branch can't land with it."
scope=
for crate in ${loop}; do
  [[ -d "crates/${crate}" ]] && scope="${scope} crates/${crate}/**"
done
known=$(cut -d' ' -f1-3 <<< "${mine}")
from_main=$(grep -v -x -F -e "${known}" <<< "${tree}")
labelled=$(
  printf '%s\n' "${mine}" "${others}"
  [[ -z "${from_main}" ]] || echo "${from_main//$'\n'/ main$'\n'} main"
)
while read -r from to dir owner; do
  [[ " ${loop} " == *" ${from} "* && " ${loop} " == *" ${to} "* ]] || continue
  checkout=${PWD}
  case "${owner}" in main | this-branch) ;; *) checkout=${worktrees}/${owner} ;; esac
  users=$(git -C "${checkout}" grep --untracked -l -w "${to//-/_}" -- "${dir}/src")
  echo "${from} > ${to} (${owner}) is used by: ${users//$'\n'/ }"
  modules=$(sed -E "s|^${dir}/src/([^/.]+).*|\1|" <<< "${users}")
  modules=$(sort -u <<< "${modules}")
  case "${modules}" in '' | *$'\n'* | lib | main) continue ;; *) ;; esac
  without=$(grep -v -x -F "${from} ${to}" <<< "${all}")
  takers=
  for folder in crates/*/; do
    crate=$(basename "${folder}")
    [[ " ${loop} " == *" ${crate} "* ]] && continue
    named=
    for word in ${modules//_/ }; do
      [[ "-${crate}-" == *"-${word}-"* ]] && named=1
    done
    [[ -n "${named}" ]] || git grep -q -w "${from//-/_}::${modules}" -- "${folder}" || continue
    LC_ALL=C tsort <<< "${without}"$'\n'"${crate} ${to}" > /dev/null 2>&1 && takers="${takers} ${crate}" && scope="${scope} ${folder}**"
  done
  [[ -z "${takers}" ]] || echo "The module ${modules} is the only reason for that edge. Crates that can take it without a loop:${takers}"
  for file in $(git grep -l -w "${from//-/_}::${modules}" -- crates tools); do
    scope="${scope} ${file}"
  done
done <<< "${labelled}"
shared=$(awk -v loop="${loop}" 'BEGIN { n = split(loop, crates, " "); for (i = 1; i <= n; i++) member[crates[i]] = 1 } ($1 in member) && !seen[$1 " " $2]++ { count[$2]++ } END { for (dep in count) if (count[dep] == n) print dep }' <<< "${all}")
shared=$(sort <<< "${shared}")
shared=${shared//$'\n'/ }
if [[ -n "${shared}" ]]; then
  echo "Every crate in the loop already depends on: ${shared}"
  for crate in ${shared}; do
    [[ -d "crates/${crate}" ]] && scope="${scope} crates/${crate}/**"
  done
fi
echo "Break the loop by moving code. Don't copy it and don't drop behaviour. If what one side needs uses nothing above a crate the whole loop already depends on, move it into that crate. If one module is the only reason for an edge, move it to a crate listed for it and fix its imports. If neither works, report blocked and say why."
covers=$(tr ' ' '\n' <<< "${scope}")
covers=$(sort -u <<< "${covers}")
echo "Scope now covers:${covers//$'\n'/ }"
exit 1
