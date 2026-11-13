#!/usr/bin/env bash
# a task file is checked only when the change adds it, since the architect amends older ones on task branches their workers can't edit
base=${PRE_COMMIT_FROM_REF:-HEAD}
specs=()
for file in "$@"; do
  if [[ ${file} == .development/checks.yaml ]] || ! git cat-file -e "${base}:${file}" 2> /dev/null; then
    specs+=("${file}")
  fi
done
[[ ${#specs[@]} -eq 0 ]] || exec cargo gate shellcheck "${specs[@]}"
