#!/usr/bin/env bash
self=$(readlink -f "$0") || exit 1
P=${self%/*}
main=${BETTERLITE_ROOT:-$(git -C "${P}" rev-parse --show-toplevel)}
worktrees=${BETTERLITE_WORKTREES:-$(dirname "${main}")/betterlite-worktrees}
L=${worktrees}/logs
maintenance() {
  if [[ -e "${main}/.git/betterlite/maintenance" ]] || [[ -e "${L}/maintenance.STOP" ]]; then
    echo "maintenance is active; no dispatch or background build started" >&2
    exit 75
  fi
}
maintenance
# Dispatches one task once a slot is free: 14 GB RAM available (from /proc/meminfo, since free prints translated labels under the flow services' LANGUAGE), disk under 95% (a full disk stops git for every flow, and a checkout takes about 0.5 GB), /tmp under 50%, fewer than 8 running dispatches waiting for an agent seat (34 sat in the Claude seat queue on 11-05 while their checkouts held disk), and memory pressure in the dispatch slice under 20% full over the last minute. Since stage 3 a dispatch builds nothing, so CPU pressure and IO stalls no longer hold a slot (Max, 11-13), but the agents still fill the slice's MemoryHigh: at 18:23 UTC on 11-13 its tasks stalled on memory 51% of the time with swap full, while MemAvailable read 22 GB, most of it reclaimable dentries and inodes from the checkouts (Max). A temp dir on disk (betterlite-worktrees/tmp/<id>) stands in for /tmp, since /tmp is RAM. There is no fixed dispatch count: the gate caps the agents on each vendor (max_parallel in agents.yaml) and overflows to the other vendor. Usage: bash next-slot.sh <id> <log name> [note]. Tests use the shared local postgres betterlite-test-postgres (127.0.0.1:55432) through POSTGRES_TEST_URL instead of a testcontainers one per test (Max).
id=$1
log=${L}/dispatch-${id}-$2.log
tools=${worktrees}/.tools
cd "${main}" || exit 1
heights=$(cd .development && LC_ALL=C perl "${P}/height.pl" "${id}") || heights="0 0"
read -r height top <<< "${heights}"
# a task within 10 steps of the longest open chain takes build slot 1, which no other task takes
lowest_slot=2
[[ "${height}" -gt 0 ]] && [[ $((height + 10)) -ge "${top}" ]] && lowest_slot=1
# bin/cargo runs these builds for `cargo gate` in task checkouts, so the gate compiles once per version instead of 625 MB in each checkout (27 GB over 44 checkouts on 11-11, Max)
gates=${worktrees}/tmp/gate
tree=$(git rev-parse HEAD:.development/gate) || exit 1
changes=$(git status --porcelain -- .development/gate) || exit 1
if [[ ! -x ${gates}/${tree}/gate ]] && [[ -z ${changes} ]]; then
  "${HOME}/.cargo/bin/cargo" build --quiet --locked --manifest-path .development/gate/Cargo.toml || exit 1
  mkdir -p "${gates}/${tree}" || exit 1
  copy=$(mktemp "${gates}/${tree}/gate.XXXXXX") || exit 1
  cp .development/gate/target/debug/gate "${copy}" && chmod 0755 "${copy}" && mv "${copy}" "${gates}/${tree}/gate" || exit 1
fi
find "${gates}" -mindepth 1 -maxdepth 1 -mmin +2880 -exec rm -rf -- {} +
slice=/sys/fs/cgroup$(systemctl --user show betterlite-dispatch.slice -p ControlGroup --value)
mkdir -p "${L}/locks"
exec 9> "${L}/locks/admission.lock"
flock 9
waiting_seats() {
  n=0
  # pgrep exits 1 when no dispatch runs, which is an empty list, not an error
  dispatches=$(pgrep -af "^([^ ]*/)?gate dispatch B[0-9]+")
  dispatches=$(grep -oE "gate dispatch B[0-9]+" <<< "${dispatches}")
  dispatches=$(sort -u <<< "${dispatches//gate dispatch /}")
  for id in ${dispatches}; do
    sandbox=
    for file in "${main}/.git/betterlite/${id}"/attempt-*.sandbox.json; do
      [[ -e "${file}" ]] || continue
      [[ -z "${sandbox}" ]] || [[ "${file}" -nt "${sandbox}" ]] && sandbox=${file}
    done
    [[ -n "${sandbox}" ]] && [[ ! -e "${sandbox%.sandbox.json}.started.json" ]] && n=$((n + 1))
  done
  echo "${n}"
}
# a reading that fails skips its check instead of holding the slot
busy() {
  memory_gb=$(awk '/^MemAvailable:/ {print int($2 / 1048576)}' /proc/meminfo) && [[ "${memory_gb}" -lt 14 ]] && return 0
  memory_full=$(awk '/^full/ {split($3, a, "="); print int(a[2])}' "${slice}/memory.pressure") && [[ "${memory_full}" -ge 20 ]] && return 0
  home_use=$(df --output=pcent /home) && home_use=${home_use##*$'\n'} && [[ "${home_use//[!0-9]/}" -ge 95 ]] && return 0
  tmp_use=$(df --output=pcent /tmp) && tmp_use=${tmp_use##*$'\n'} && [[ "${tmp_use//[!0-9]/}" -ge 50 ]] && return 0
  seats=$(waiting_seats) && [[ "${seats}" -ge 8 ]] && return 0
  return 1
}
while busy; do
  flock -u 9
  maintenance
  sleep 30
  flock 9
done
maintenance
tmp=${worktrees}/tmp/${id}
mkdir -p "${tmp}"
# Only a newly started cache server changes its capacity. Existing clients keep their server.
if ! pgrep -x sccache > /dev/null; then
  mkdir -p "${worktrees}/tmp/sccache"
  cache_bytes=$(du -sb "${HOME}/.cache/sccache") || exit 1
  cache_bytes=${cache_bytes%%$'\t'*}
  free_bytes=$(df --output=avail -B1 "${worktrees}") || exit 1
  free_bytes=${free_bytes##*$'\n'}
  growth=$((free_bytes > 64 * 1024 ** 3 ? free_bytes - 64 * 1024 ** 3 : 0))
  cache_size=$((cache_bytes + growth))
  ((cache_size > 24 * 1024 ** 3)) && cache_size=$((24 * 1024 ** 3))
  ((cache_size < cache_bytes)) && cache_size=${cache_bytes}
  TMPDIR=${worktrees}/tmp/sccache SCCACHE_CACHE_SIZE=${cache_size} SCCACHE_IDLE_TIMEOUT=0 \
    systemd-run --user --scope --slice=betterlite-dispatch.slice -q -- nice -n 10 "${tools}/bin/sccache" --start-server > /dev/null 2>&1 || exit 1
fi
nc -z -w 5 127.0.0.1 55432 || {
  echo "test database unavailable; no worker started" >&2
  exit 1
}
socket_dir=$(mktemp -d /tmp/bl-pg-XXXXXXXX) || exit 1
socket=${socket_dir}/.s.PGSQL.55432
setsid setpriv --pdeathsig TERM -- socat "UNIX-LISTEN:${socket},fork,max-children=64,mode=0600" TCP4:127.0.0.1:55432,connect-timeout=5 &
relay=$!
trap 'kill -TERM -- "-$relay" 2>/dev/null; wait "$relay" 2>/dev/null; rm -rf -- "$socket_dir"' EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
waited=0
while [[ "${waited}" -lt 100 ]] && [[ ! -S "${socket}" ]]; do
  kill -0 "${relay}" 2> /dev/null || exit 1
  sleep 0.05
  waited=$((waited + 1))
done
[[ -S "${socket}" ]] || {
  echo "test database socket unavailable; no worker started" >&2
  exit 1
}
UV_CACHE_DIR="${HOME}/.cache/uv" GATE_POSTGRES_SOCKET="${socket}" POSTGRES_TEST_URL="postgresql://betterlite:betterlite@127.0.0.1:55432/betterlite?host=${socket_dir}" TMPDIR=${tmp} PATH=${P}/bin:${HOME}/.local/bin:${tools}/bin:${PATH} \
  NEXTEST_TEST_THREADS=2 GATE_BUILD_SLOTS=8 GATE_LOWEST_BUILD_SLOT=${lowest_slot} GATE_SHARED_BUILDS=${gates} \
  systemd-run --user --scope --slice=betterlite-dispatch.slice -p MemoryMax=16G -p MemorySwapMax=4G -q -- bash "${P}/build-env.sh" nice -n 10 cargo gate dispatch "${id}" ${3:+--note "$3"} ${GRANT:+--grant "${GRANT}"} > "${log}" 2>&1 9>&- &
child=$!
while kill -0 "${child}" 2> /dev/null && ! pgrep -f "^([^ ]*/)?gate dispatch ${id}( |$)" > /dev/null; do
  sleep 0.05
done
flock -u 9
wait "${child}"
code=$?
echo "${id} exit ${code}" >> "${log}"
tail -2 "${log}"
exit "${code}"
