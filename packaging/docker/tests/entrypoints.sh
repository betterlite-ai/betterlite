#!/bin/bash
set -u

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir "${work}/bin" "${work}/cwd"

for name in betterlite betterlite-db-admin; do
  cat > "${work}/bin/${name}" << 'STUB'
#!/bin/sh
echo "$$" > "$STUB_DIR/stub.pid"
printf '%s' "${DD_TRACE_OPENAI_ENABLED-<unset>}" > "$STUB_DIR/ddtrace-env"
for arg in "$@"; do
    printf '%s\n' "$arg"
done
exit "${STUB_EXIT:-0}"
STUB
  chmod +x "${work}/bin/${name}"
done
export PATH="${work}/bin:${PATH}"
export STUB_DIR="${work}"

fail() {
  echo "FAIL: $1" >&2
  exit 1
}

expect_equal() {
  [[ "$2" = "$3" ]] || fail "$1: expected [$2] got [$3]"
}

# Command substitution drops trailing newlines, so exact output is compared as files.
expect_lines() {
  local name=$1 file=$2
  shift 2
  printf '%s\n' "$@" > "${work}/expected"
  cmp -s "${work}/expected" "${file}" || fail "${name}: expected [$(< "${work}/expected")] got [$(< "${file}")]"
}

expect_empty() {
  [[ ! -s "$2" ]] || fail "$1: expected no output, got [$(< "$2")]"
}

# Runs the command in the background so $child is its pid; an exec keeps that pid for the stub.
run() {
  rm -f "${work}/stub.pid" "${work}/ddtrace-env"
  "$@" > "${work}/out" 2> "${work}/err" &
  child=$!
  wait "${child}"
  code=$?
  stub_pid="$(cat "${work}/stub.pid" 2> /dev/null || true)"
}

ddtrace_line="USE_DDTRACE is ignored because the native server has no ddtrace support"

cd "${work}" || fail "cannot enter the scratch folder"

for script in prod_entrypoint.sh component_entrypoint.sh entrypoint.sh build_admin_ui.sh; do
  [[ -f "${here}/${script}" ]] || fail "${script} is missing"
done

unset USE_DDTRACE PROMETHEUS_MULTIPROC_DIR DD_TRACE_OPENAI_ENABLED

run sh "${here}/prod_entrypoint.sh" --port 4000 "two words" ""
expect_lines "prod args" "${work}/out" --port 4000 "two words" ""
expect_equal "prod exit" 0 "${code}"
expect_empty "prod unset prints nothing" "${work}/err"
expect_equal "prod exec keeps the process" "${child}" "${stub_pid}"

run sh "${here}/prod_entrypoint.sh"
expect_empty "prod no args" "${work}/out"
expect_equal "prod no args exit" 0 "${code}"
expect_empty "prod no args prints nothing" "${work}/err"
[[ -n "${stub_pid}" ]] || fail "prod no args did not run the server"

STUB_EXIT=7 run sh "${here}/prod_entrypoint.sh" --port 1
expect_equal "prod exit code" 7 "${code}"

for value in true TRUE True tRuE tRUE; do
  USE_DDTRACE=${value} run sh "${here}/prod_entrypoint.sh" --port 4000
  expect_lines "prod ddtrace ${value} line" "${work}/err" "${ddtrace_line}"
  expect_lines "prod ddtrace ${value} args" "${work}/out" --port 4000
  expect_equal "prod ddtrace ${value} exec" "${child}" "${stub_pid}"
  expect_equal "prod ddtrace ${value} leaves DD_TRACE_OPENAI_ENABLED" "<unset>" "$(< "${work}/ddtrace-env")"
done

DD_TRACE_OPENAI_ENABLED=keep USE_DDTRACE=true run sh "${here}/prod_entrypoint.sh" --port 4000
expect_equal "prod keeps caller DD_TRACE_OPENAI_ENABLED" "keep" "$(< "${work}/ddtrace-env")"

for value in false 1 yes "" truee; do
  USE_DDTRACE=${value} run sh "${here}/prod_entrypoint.sh" --port 4000
  expect_empty "prod ddtrace [${value}] silent" "${work}/err"
  expect_lines "prod ddtrace [${value}] runs server" "${work}/out" --port 4000
done

touch "${work}/keep-unset.db"
run sh "${here}/component_entrypoint.sh" betterlite --flag "a b"
expect_lines "component args" "${work}/out" --flag "a b"
expect_equal "component exec keeps the process" "${child}" "${stub_pid}"
expect_empty "component unset silent" "${work}/err"
[[ -e "${work}/keep-unset.db" ]] || fail "unset variable removed a file from the working directory"

STUB_EXIT=3 run sh "${here}/component_entrypoint.sh" betterlite
expect_equal "component exit code" 3 "${code}"

for value in true TRUE True tRuE tRUE; do
  USE_DDTRACE=${value} run sh "${here}/component_entrypoint.sh" betterlite --x
  expect_lines "component ddtrace ${value} line" "${work}/err" "${ddtrace_line}"
  expect_lines "component ddtrace ${value} command" "${work}/out" --x
  expect_equal "component ddtrace ${value} leaves DD_TRACE_OPENAI_ENABLED" "<unset>" "$(< "${work}/ddtrace-env")"
done

for value in false 1 yes ""; do
  USE_DDTRACE=${value} run sh "${here}/component_entrypoint.sh" betterlite --x
  expect_empty "component ddtrace [${value}] silent" "${work}/err"
  expect_lines "component ddtrace [${value}] command" "${work}/out" --x
done

metrics="${work}/prom dir"
mkdir -p "${metrics}/nested.db"
touch "${metrics}/a.db" "${metrics}/b.db" "${metrics}/keep.txt" "${metrics}/db" "${metrics}/c.db.bak"
PROMETHEUS_MULTIPROC_DIR=${metrics} run sh "${here}/component_entrypoint.sh" betterlite --y
expect_equal "cleanup exit" 0 "${code}"
expect_lines "cleanup command ran" "${work}/out" --y
[[ ! -e "${metrics}/a.db" ]] || fail "a.db survived"
[[ ! -e "${metrics}/b.db" ]] || fail "b.db survived"
[[ -e "${metrics}/keep.txt" ]] || fail "keep.txt was removed"
[[ -e "${metrics}/db" ]] || fail "db was removed"
[[ -e "${metrics}/c.db.bak" ]] || fail "c.db.bak was removed"
[[ -d "${metrics}/nested.db" ]] || fail "nested.db directory was removed"

PROMETHEUS_MULTIPROC_DIR="${work}/created" run sh "${here}/component_entrypoint.sh" betterlite
expect_equal "created folder exit" 0 "${code}"
[[ -d "${work}/created" ]] || fail "missing folder was not created"
expect_empty "created folder passes no argument" "${work}/out"
[[ -n "${stub_pid}" ]] || fail "created folder did not run the command"

# A glob that matches no file must not fail the script.
mkdir "${work}/no-db"
touch "${work}/no-db/keep.txt"
PROMETHEUS_MULTIPROC_DIR="${work}/no-db" run sh "${here}/component_entrypoint.sh" betterlite
expect_equal "folder without db files exit" 0 "${code}"
[[ -e "${work}/no-db/keep.txt" ]] || fail "keep.txt was removed from a folder without db files"
expect_empty "folder without db files silent" "${work}/err"
expect_empty "folder without db files passes no argument" "${work}/out"
[[ -n "${stub_pid}" ]] || fail "folder without db files did not run the command"

touch "${work}/cwd/keep.db"
cd "${work}/cwd" || fail "cannot enter the scratch folder"
PROMETHEUS_MULTIPROC_DIR='' run sh "${here}/component_entrypoint.sh" betterlite
expect_equal "empty variable exit" 0 "${code}"
listing=$(ls -A "${work}/cwd") || fail "cannot list the scratch folder"
expect_equal "empty variable leaves the folder" "keep.db" "${listing}"
expect_empty "empty variable silent" "${work}/err"
expect_empty "empty variable passes no argument" "${work}/out"
[[ -n "${stub_pid}" ]] || fail "empty variable did not run the command"
rm "${work}/cwd/keep.db"

run bash "${here}/entrypoint.sh"
expect_lines "migrate output" "${work}/out" migrate "Migration script ran successfully!"
expect_equal "migrate exit" 0 "${code}"
expect_empty "migrate silent" "${work}/err"

run bash "${here}/entrypoint.sh" --config "my config.yaml"
expect_lines "migrate extra args" "${work}/out" migrate --config "my config.yaml" "Migration script ran successfully!"

for status in 1 2; do
  STUB_EXIT=${status} run bash "${here}/entrypoint.sh" --config c.yaml
  expect_equal "migrate exit ${status}" "${status}" "${code}"
  expect_lines "migrate ${status} output" "${work}/out" migrate --config c.yaml
done

for variant in absent present; do
  rm -rf "${work}/cwd"
  mkdir "${work}/cwd"
  cd "${work}/cwd" || fail "cannot enter the scratch folder"
  if [[ "${variant}" = present ]]; then
    mkdir -p enterprise/enterprise_ui
    echo '{}' > enterprise/enterprise_ui/enterprise_colors.json
  fi
  # An empty PATH proves the script runs no external command; pwd and echo are builtins.
  PATH="${work}/none" /bin/bash "${here}/build_admin_ui.sh" > "${work}/out" 2> "${work}/err"
  expect_equal "admin ui ${variant} exit" 0 "$?"
  expect_lines "admin ui ${variant} output" "${work}/out" "" "${work}/cwd" "Admin UI - using default LiteLLM UI"
  expect_empty "admin ui ${variant} stderr" "${work}/err"
done

echo ok
