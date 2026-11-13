#!/usr/bin/env bash
# dispatches and clippy-seed.sh build through this, since cargo rebuilds every unit whose profile or wrapper settings differ from the copied ones
self=$(readlink -f "$0") || exit 1
P=${self%/*}
main=${BETTERLITE_ROOT:-$(git -C "${P}" rev-parse --show-toplevel)}
worktrees=${BETTERLITE_WORKTREES:-$(dirname "${main}")/betterlite-worktrees}
export RUSTC_WRAPPER=${worktrees}/.tools/bin/sccache SCCACHE_IGNORE_SERVER_IO_ERROR=1 CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=line-tables-only CARGO_PROFILE_TEST_DEBUG=line-tables-only CARGO_BUILD_JOBS=2
exec "$@"
