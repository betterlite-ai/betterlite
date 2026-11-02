#!/usr/bin/env bash
source "${BETTERLITE_OPS:-/home/user/Desktop/betterlite/.development/ops}/common.sh"
# Dispatches one task once a slot is free (CPU under 85% busy (the slice caps dispatches at 1440%, 90% of the 16 cores (Max)), 10 GB RAM free, disk under 83%, leaving running builds room under 90%; caches are retained until explicit cleanup), with lean cached builds and a temp dir on disk (betterlite-worktrees/tmp/<id>), since /tmp is RAM. The gate caps the agents on each vendor (max_parallel in agents.yaml), so there is no fixed dispatch count here. Usage: bash next-slot3.sh <id> <log name> [note] Tests use the shared local postgres betterlite-test-postgres (127.0.0.1:55432) through POSTGRES_TEST_URL instead of a testcontainers one per test (Max). A new worktree gets the third-party artifacts of the oracle warm-up target (seed-target.sh), and the task being dispatched keeps its own target.
id=$1
log=$L/dispatch-$id-$2.log
tools=$worktrees/.tools
cd "$main" || exit 1
mkdir -p "$L/locks"
exec 9> "$L/locks/admission.lock"
flock 9
busy() {
  read -r _ u n s i w q sq st _ < /proc/stat
  sleep 1
  read -r _ u2 n2 s2 i2 w2 q2 sq2 st2 _ < /proc/stat
  busy=$(( (u2 + n2 + s2 + q2 + sq2 + st2) - (u + n + s + q + sq + st) ))
  [ $(( busy * 100 / (busy + i2 + w2 - i - w) )) -ge 85 ] && return 0
  [ "$(cut -d" " -f2 /proc/loadavg | cut -d. -f1)" -ge 16 ] && return 0
  [ "$(free -g | awk '/^Mem:/ {print $7}')" -lt 10 ] && return 0
  [ "$(df --output=pcent /home | tail -1 | tr -dc 0-9)" -ge 83 ] && return 0
  [ "$(df --output=pcent /tmp | tail -1 | tr -dc 0-9)" -ge 50 ] && return 0
  return 1
}
while busy; do
  flock -u 9
  maintenance
  sleep 30
  flock 9
done
maintenance
tmp=$worktrees/tmp/$id
mkdir -p "$tmp"
# Only a newly started cache server changes its capacity. Existing clients keep their server.
if ! pgrep -x sccache > /dev/null; then
  mkdir -p "$worktrees/tmp/sccache"
  cache_size=$(uv run --locked --project "$ops" python "$ops/cache_policy.py" "$worktrees" "$HOME/.cache/sccache") || exit 1
  TMPDIR=$worktrees/tmp/sccache SCCACHE_CACHE_SIZE=$cache_size SCCACHE_IDLE_TIMEOUT=0 \
    systemd-run --user --scope --slice=betterlite-dispatch.slice -q -- nice -n 10 "$tools/bin/sccache" --start-server > /dev/null 2>&1 || exit 1
fi
POSTGRES_TEST_URL=postgresql://betterlite:betterlite@127.0.0.1:55432/betterlite TMPDIR=$tmp PATH=$tools/bin:$PATH RUSTC_WRAPPER=$tools/bin/sccache SCCACHE_IGNORE_SERVER_IO_ERROR=1 CARGO_INCREMENTAL=0 \
  CARGO_PROFILE_DEV_DEBUG=line-tables-only CARGO_PROFILE_TEST_DEBUG=line-tables-only CARGO_BUILD_JOBS=3 NEXTEST_TEST_THREADS=2 GATE_BUILD_SLOTS=4 \
  systemd-run --user --scope --slice=betterlite-dispatch.slice -p MemoryMax=16G -p MemorySwapMax=4G -q -- nice -n 10 uv run --locked --project "$ops" python "$ops/postgres_bridge.py" -- cargo gate dispatch "$id" ${3:+--note "$3"} ${GRANT:+--grant "$GRANT"} > "$log" 2>&1 9>&- &
child=$!
setsid -f bash /tmp/claude-1000/-home-user-Desktop-betterlite/b70e8a95-be32-4db9-80ae-d4296390a241/scratchpad/prep/seed-target.sh "$id" > /dev/null 2>&1 9>&-
flock -u 9
wait $child
code=$?
echo "$id exit $code" >> "$log"
tail -2 "$log"
exit $code
