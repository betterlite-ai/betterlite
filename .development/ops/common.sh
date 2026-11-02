#!/usr/bin/env bash
main=${BETTERLITE_ROOT:-/home/user/Desktop/betterlite}
worktrees=${BETTERLITE_WORKTREES:-/home/user/Desktop/betterlite-worktrees}
ops=${BETTERLITE_OPS:-$main/.development/ops}
L=$worktrees/logs
maintenance() {
  if [ -e "$main/.git/betterlite/maintenance" ] || [ -e "$L/maintenance.STOP" ]; then
    echo "maintenance is active; no dispatch or background build started" >&2
    exit 75
  fi
}
maintenance
