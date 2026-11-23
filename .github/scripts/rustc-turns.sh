#!/usr/bin/env bash
name=
kind=lib
for ((i = 1; i <= ${#}; i++)); do
  case ${!i} in
    --crate-name)
      next=$((i + 1))
      name=${!next}
      ;;
    --test) kind="test" ;;
    *) ;;
  esac
done
# the builds that peak over 4 GB at four jobs (11-23), so a 16 GB runner builds one of them at a time
case ${name}:${kind} in
  betterlite_server:test | betterlite_core:test | betterlite_logging:test | betterlite_db:test | betterlite_guardrails:test | betterlite_config:test | betterlite_openai:test | betterlite_oracle:test | betterlite_oracle:lib | betterlite_config:lib | betterlite_server:lib)
    exec flock /tmp/betterlite-heavy-rustc.lock "${@}"
    ;;
  *) ;;
esac
exec "${@}"
