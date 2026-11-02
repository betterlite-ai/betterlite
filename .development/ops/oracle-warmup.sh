#!/usr/bin/env bash
source "${BETTERLITE_OPS:-/home/user/Desktop/betterlite/.development/ops}/common.sh"
# Fills ~/.cache/betterlite-oracle with LiteLLM helper answers computed from origin/main, in a worktree detached at main. When helper inputs or cases change it runs the oracle tests with BETTERLITE_ORACLE_CACHE=shared inside betterlite-dispatch.slice, so the slice's 1440% cap covers it and a full slice only slows it down; it still waits for 10 GB RAM free and disk under 83%. Stops when logs/oracle-warmup.STOP exists. Usage: setsid -f bash oracle-warmup.sh It builds with sccache, no incremental data and line-tables-only debuginfo, like the dispatches, so its target stays small.
w=$worktrees/oracle-warmup
shared="$HOME/.cache/betterlite-oracle"
busy() {
  [ "$(free -g | awk '/^Mem:/ {print $7}')" -lt 10 ] && return 0
  [ "$(df --output=pcent /home | tail -1 | tr -dc 0-9)" -ge 83 ] && return 0
  return 1
}
last=$(cat "$L/oracle-warmup.inputs" 2>/dev/null)
while [ ! -e "$L/oracle-warmup.STOP" ]; do
  maintenance
  git -C "$w" fetch -q origin main 2>/dev/null || { sleep 60; continue; }
  sha=$(git -C "$w" rev-parse origin/main)
  inputs=$(uv run --locked --project "$ops" python "$ops/warm_inputs.py" "$w" "$sha") || exit 1
  if [ "$inputs" != "$last" ] && ! busy; then
    maintenance
    [ -z "$(git -C "$w" status --porcelain)" ] || { echo "warm-up checkout is dirty" >&2; exit 1; }
    git -C "$w" checkout -q --detach "$sha"
    before=$(ls "$shared" 2>/dev/null | wc -l)
    started=$(date +%s)
    if (cd "$w" && POSTGRES_TEST_URL=postgresql://betterlite:betterlite@127.0.0.1:55432/betterlite BETTERLITE_ORACLE_CACHE=shared systemd-run --user --scope --slice=betterlite-dispatch.slice -q -- nice -n 19 env RUSTC_WRAPPER=/home/user/Desktop/betterlite-worktrees/.tools/bin/sccache SCCACHE_IGNORE_SERVER_IO_ERROR=1 CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=line-tables-only CARGO_PROFILE_TEST_DEBUG=line-tables-only cargo nextest run --locked --build-jobs 2 --test-threads 2 -p betterlite-oracle > "$L/oracle-warmup.log" 2>&1); then
      last=$inputs
      printf '%s\n' "$last" > "$L/oracle-warmup.inputs.part"
      mv "$L/oracle-warmup.inputs.part" "$L/oracle-warmup.inputs"
      echo "$(date +%H:%M) oracle warm-up at ${sha:0:8}: $(( $(ls "$shared" | wc -l) - before )) new answers in $(( $(date +%s) - started )) s" >> "$L/flow-events.log"
    else
      echo "$(date +%H:%M) oracle warm-up at ${sha:0:8} failed, see logs/oracle-warmup.log" >> "$L/flow-events.log"
    fi

  fi
  sleep 300
done
