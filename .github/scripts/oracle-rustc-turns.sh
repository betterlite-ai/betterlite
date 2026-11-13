#!/usr/bin/env bash
for ((i = 1; i < ${#}; i++)); do
  if [[ ${!i} == --crate-name ]]; then
    next=$((i + 1))
    if [[ ${!next} == betterlite_oracle ]]; then
      exec flock /tmp/betterlite-oracle-rustc.lock "${@}"
    fi
    break
  fi
done
exec "${@}"
