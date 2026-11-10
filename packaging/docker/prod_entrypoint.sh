#!/bin/sh

case "$USE_DDTRACE" in
    [Tt][Rr][Uu][Ee])
        echo "USE_DDTRACE is ignored because the native server has no ddtrace support" >&2
        ;;
esac

exec betterlite "$@"
