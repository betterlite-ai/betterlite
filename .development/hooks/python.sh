#!/usr/bin/env bash
# prek passes the changed .py files; a file in python-legacy.txt keeps only the checks it had before these, until a cleanup task fixes it and takes it off the list
mode=$1
shift
strict=()
legacy=()
for file in "$@"; do
  if grep -q -x -F -- "${file}" python-legacy.txt; then
    legacy+=("${file}")
  else
    strict+=("${file}")
  fi
done
oracle=(uv run --locked --project tools/oracle/litellm)
case ${mode} in
  ruff)
    files=("${strict[@]}")
    for file in "${legacy[@]}"; do
      [[ ${file} == tools/oracle/helpers/* ]] && files+=("${file}")
    done
    [[ ${#files[@]} -eq 0 ]] || exec "${oracle[@]}" ruff check --force-exclude "${files[@]}"
    ;;
  rules)
    [[ ${#strict[@]} -eq 0 ]] && exit 0
    status=0
    ast-grep scan --config .ast-grep/python/sgconfig.yml "${strict[@]}" || status=1
    suppressed=$(grep -n -H -E '(^|[^[:alnum:]_])cast\(|#[[:space:]]*(noqa|type:[[:space:]]*ignore|pyright:[[:space:]]*ignore\[)' -- "${strict[@]}")
    unlinked=
    owned=
    if [[ -n ${suppressed} ]]; then
      unlinked=$(grep -v -E 'https://github\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/blob/[0-9a-f]{40}/' <<< "${suppressed}")
      owned=$(grep -E 'github\.com/betterlite-ai/' <<< "${suppressed}")
    fi
    # mypy never reports pyright's private-usage and unknown-member errors, so a pinned pyright: ignore[rule] passes above like a type: ignore
    banned=$(awk '{ line = $0; gsub(/#[[:space:]]*pyright:[[:space:]]*ignore\[[A-Za-z, ]+\]/, "", line) } line ~ /#[[:space:]]*(pyright:|mypy:|ruff:[[:space:]]*noqa)/ { print FILENAME ":" FNR ":" $0 }' "${strict[@]}")
    for found in "${unlinked}" "${owned}"; do
      [[ -z ${found} ]] && continue
      printf '%s\n' "${found}"
      echo "a cast, noqa, type: ignore or pyright: ignore[rule] is allowed only for an upstream dependency's own types, and its line carries the upstream source it works around as a link pinned to a commit (https://github.com/<owner>/<repo>/blob/<40-hex sha>/...), and each such line costs one from the task's max_new_expects" >&2
      status=1
    done
    if [[ -n ${banned} ]]; then
      printf '%s\n' "${banned}"
      echo "mypy comments, a pyright comment other than ignore[rule] and a file-wide ruff noqa have no upstream case; fix the code" >&2
      status=1
    fi
    for file in "${strict[@]}"; do
      limit=300
      [[ ${file} == */tests/* ]] && limit=1000
      lines=$(wc -l < "${file}") || exit 1
      if [[ ${lines} -gt ${limit} ]]; then
        echo "${file}: ${lines} lines, past the ${limit}-line limit; split it along a real boundary" >&2
        status=1
      fi
    done
    exit "${status}"
    ;;
  mypy)
    [[ ${#strict[@]} -eq 0 ]] || exec "${oracle[@]}" mypy --config-file tools/oracle/litellm/mypy.toml "${strict[@]}"
    ;;
  pyright)
    # 1031 task checks run basedpyright over the whole helpers folder, so a helper change has to pass that same run, importers included
    exec "${oracle[@]}" basedpyright --project tools/oracle/helpers/pyrightconfig.json
    ;;
  pyright-files)
    exec "${oracle[@]}" basedpyright --project tools/oracle/helpers/pyrightconfig.json "$@"
    ;;
  similarity)
    tracked=$(git ls-files -- '*.py' ':!:**/fixtures/**') || exit 1
    files=()
    while read -r file; do
      grep -q -x -F -- "${file}" python-legacy.txt || files+=("${file}")
    done <<< "${tracked}"
    [[ ${#files[@]} -eq 0 ]] || exec similarity-py "${files[@]}" --threshold 0.85 --min-lines 8 --fail-on-duplicates
    ;;
  *)
    echo "usage: python.sh ruff|rules|mypy|pyright|similarity FILE..." >&2
    exit 2
    ;;
esac
