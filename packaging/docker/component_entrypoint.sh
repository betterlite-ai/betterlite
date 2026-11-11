#!/bin/sh

# stale samples from a previous container incarnation would be summed into the aggregate
if [ -n "${PROMETHEUS_MULTIPROC_DIR}" ]; then
  mkdir -p "${PROMETHEUS_MULTIPROC_DIR}"
  rm -f "${PROMETHEUS_MULTIPROC_DIR}"/*.db
fi

case "${USE_DDTRACE:-}" in
  [Tt][Rr][Uu][Ee])
    echo "USE_DDTRACE is ignored because the native server has no ddtrace support" >&2
    ;;
  # unset or any other value starts the command without a word
  *) ;;
esac

exec "$@"
