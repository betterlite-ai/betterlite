#!/usr/bin/env bash
self=$(readlink -f "$0") || exit 1
P=${self%/*}
main=${BETTERLITE_ROOT:-$(git -C "${P}" rev-parse --show-toplevel)}
worktrees=${BETTERLITE_WORKTREES:-$(dirname "${main}")/betterlite-worktrees}
L=${worktrees}/logs
export BETTERLITE_LOGS=${L}
events=${L}/flow-events.log
cap=${BETTERLITE_MAX_FLOWS:-40}
mkdir -p "${L}/flows"
while [[ ! -f "${L}/feeder.STOP" ]]; do
  while [[ -e "${main}/.git/betterlite/maintenance" ]] || [[ -e "${L}/maintenance.STOP" ]]; do sleep 60; done
  git -C "${main}" pull -q --ff-only 2> /dev/null
  if ! (cd "${main}" && cargo gate availability) > "${L}/availability-feeder.log" 2>&1; then
    sleep 60
    continue
  fi
  # pgrep exits 1 when no flow runs, which is an empty list, not an error
  flows=$(pgrep -f -a '^bash .*/dispatch-flow\.sh B[0-9]+ ')
  flows=$(grep -o -E 'dispatch-flow\.sh B[0-9]+' <<< "${flows}")
  flows=$(sort -u <<< "${flows//dispatch-flow.sh /}")
  # a merged task's flow has freed its checkout and only waits for main's run, so it takes no slot and blocks no dependent
  open=$(for id in ${flows}; do
    grep -q '^status: done' "${main}/.development/backlog/${id}.yaml" 2> /dev/null || echo "${id}"
  done)
  count=$(grep -c . <<< "${open}")
  journals=$(for journal in "${main}"/.git/betterlite/B*; do
    j=${journal##*/}
    [[ ${j} =~ ^B[0-9]+$ ]] || continue
    grep -q '^status: done' "${main}/.development/backlog/${j}.yaml" 2> /dev/null || echo "${j}"
  done)
  held=$(grep . <<< "${open}"$'\n'"${journals}")
  held=$(sort -u <<< "${held}")
  waiting=0
  slots=$(pgrep -f -a '^bash .*/next-slot[0-9]*\.sh B[0-9]+ ')
  slots=$(awk '{print $4}' <<< "${slots}")
  slots=$(sort -u <<< "${slots}")
  for id in ${slots}; do
    pgrep -f "gate dispatch ${id}( |$)" > /dev/null || waiting=$((waiting + 1))
  done
  room=2
  [[ "${waiting}" -ge 2 ]] && room=0
  [[ $((cap - count)) -lt "${room}" ]] && room=$((cap - count))
  # a flow that died without finishing left its arguments behind, oldest first
  stale=$(find "${L}/flows" -maxdepth 1 -name '*.args' -printf '%T@ %p\n')
  stale=$(LC_ALL=C sort -n <<< "${stale}")
  stale=$(cut -d' ' -f2- <<< "${stale}")
  for args in ${stale}; do
    [[ "${room}" -gt 0 ]] || break
    id=$(basename "${args}" .args)
    grep -q -x -F "${id}" <<< "${flows}" && continue
    if grep -q '^status: done' "${main}/.development/backlog/${id}.yaml" 2> /dev/null; then
      rm -f -- "${args}"
      continue
    fi
    if ! run=$(jq -e -r .run "${args}" 2> /dev/null); then
      mv -- "${args}" "${args}.unread"
      printf '%(%H:%M)T %s\n' -1 "${id} has an args file the feeder can't read, moved to ${args}.unread for the architect" >> "${events}"
      continue
    fi
    note=$(jq -r .note "${args}")
    grant=$(jq -r .grant "${args}")
    ci_rounds=$(jq -r .ci_rounds "${args}")
    conflicts=$(jq -r .conflicts "${args}")
    land_only=$(jq -r .land_only "${args}")
    printf '%(%H:%M)T %s\n' -1 "${id} resumed by the feeder (${run})" >> "${events}"
    GRANT=${grant} CI_ROUNDS=${ci_rounds} CONFLICTS=${conflicts} LAND_ONLY=${land_only} setsid -f bash "${P}/dispatch-flow.sh" "${id}" "${run}" "${note}" > /dev/null 2>&1 < /dev/null
    room=$((room - 1))
    sleep 5
  done
  if [[ "${room}" -gt 0 ]]; then
    # free.pl marks each held id in progress, so the ids go in as separate arguments
    read -r -d '' -a held_ids <<< "${held}"
    # free.pl failing leaves no pick this round
    picks=$(cd "${main}/.development" && perl "${P}/free.pl" 1 "${held_ids[@]}")
    picks=$(awk '$1 == "free" {print $2}' <<< "${picks}")
    picks=$(grep -v -x -F -m "${room}" "${held:-none}" <<< "${picks}")
    for id in ${picks}; do
      printf '%(%H:%M)T %s\n' -1 "${id} picked by the feeder" >> "${events}"
      setsid -f bash "${P}/dispatch-flow.sh" "${id}" run1 > /dev/null 2>&1 < /dev/null
      sleep 5
    done
  fi
  sleep 120
done
printf '%(%H:%M)T %s\n' -1 "feeder stopped" >> "${events}"
