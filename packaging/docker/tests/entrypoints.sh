#!/bin/bash
set -u

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir "$work/bin" "$work/cwd"

for name in betterlite betterlite-db-admin; do
    cat > "$work/bin/$name" <<'STUB'
#!/bin/sh
echo "$$" > "$STUB_DIR/stub.pid"
for arg in "$@"; do
    printf '%s\n' "$arg"
done
exit "${STUB_EXIT:-0}"
STUB
    chmod +x "$work/bin/$name"
done
export PATH="$work/bin:$PATH"
export STUB_DIR="$work"

fail() {
    echo "FAIL: $1" >&2
    exit 1
}

expect_equal() {
    [ "$2" = "$3" ] || fail "$1: expected [$2] got [$3]"
}

# Runs the command in the background so $child is its pid; an exec keeps that pid for the stub.
run() {
    rm -f "$work/stub.pid"
    "$@" > "$work/out" 2> "$work/err" &
    child=$!
    wait "$child"
    code=$?
    stub_pid="$(cat "$work/stub.pid" 2>/dev/null || true)"
}

ddtrace_line="USE_DDTRACE is ignored because the native server has no ddtrace support"
nl=$'\n'

for script in prod_entrypoint.sh component_entrypoint.sh entrypoint.sh build_admin_ui.sh; do
    [ -f "$here/$script" ] || fail "$script is missing"
done

unset USE_DDTRACE PROMETHEUS_MULTIPROC_DIR

run sh "$here/prod_entrypoint.sh" --port 4000 "two words" ""
printf '%s\n' --port 4000 "two words" "" > "$work/expected"
cmp -s "$work/expected" "$work/out" || fail "prod args differ from the container command"
expect_equal "prod exit" 0 "$code"
expect_equal "prod unset prints nothing" "" "$(cat "$work/err")"
expect_equal "prod exec keeps the process" "$child" "$stub_pid"

run sh "$here/prod_entrypoint.sh"
expect_equal "prod no args" "" "$(cat "$work/out")"
expect_equal "prod no args exit" 0 "$code"
[ -n "$stub_pid" ] || fail "prod no args did not run the server"

STUB_EXIT=7 run sh "$here/prod_entrypoint.sh" --port 1
expect_equal "prod exit code" 7 "$code"

for value in true TRUE True tRuE; do
    USE_DDTRACE=$value run sh "$here/prod_entrypoint.sh" --port 4000
    expect_equal "prod ddtrace $value line" "$ddtrace_line" "$(cat "$work/err")"
    expect_equal "prod ddtrace $value args" "--port${nl}4000" "$(cat "$work/out")"
    expect_equal "prod ddtrace $value exec" "$child" "$stub_pid"
done

for value in false 1 yes "" truee; do
    USE_DDTRACE=$value run sh "$here/prod_entrypoint.sh" --port 4000
    expect_equal "prod ddtrace [$value] silent" "" "$(cat "$work/err")"
    expect_equal "prod ddtrace [$value] runs server" "--port${nl}4000" "$(cat "$work/out")"
done

run sh "$here/component_entrypoint.sh" betterlite --flag "a b"
expect_equal "component args" "--flag${nl}a b" "$(cat "$work/out")"
expect_equal "component exec keeps the process" "$child" "$stub_pid"
expect_equal "component unset silent" "" "$(cat "$work/err")"

STUB_EXIT=3 run sh "$here/component_entrypoint.sh" betterlite
expect_equal "component exit code" 3 "$code"

for value in true TRUE True tRuE; do
    USE_DDTRACE=$value run sh "$here/component_entrypoint.sh" betterlite --x
    expect_equal "component ddtrace $value line" "$ddtrace_line" "$(cat "$work/err")"
    expect_equal "component ddtrace $value command" "--x" "$(cat "$work/out")"
done

for value in false 1 yes ""; do
    USE_DDTRACE=$value run sh "$here/component_entrypoint.sh" betterlite --x
    expect_equal "component ddtrace [$value] silent" "" "$(cat "$work/err")"
    expect_equal "component ddtrace [$value] command" "--x" "$(cat "$work/out")"
done

metrics="$work/prom dir"
mkdir -p "$metrics/nested.db"
touch "$metrics/a.db" "$metrics/b.db" "$metrics/keep.txt" "$metrics/db" "$metrics/c.db.bak"
PROMETHEUS_MULTIPROC_DIR=$metrics run sh "$here/component_entrypoint.sh" betterlite --y
expect_equal "cleanup exit" 0 "$code"
expect_equal "cleanup command ran" "--y" "$(cat "$work/out")"
[ ! -e "$metrics/a.db" ] || fail "a.db survived"
[ ! -e "$metrics/b.db" ] || fail "b.db survived"
[ -e "$metrics/keep.txt" ] || fail "keep.txt was removed"
[ -e "$metrics/db" ] || fail "db was removed"
[ -e "$metrics/c.db.bak" ] || fail "c.db.bak was removed"
[ -d "$metrics/nested.db" ] || fail "nested.db directory was removed"

PROMETHEUS_MULTIPROC_DIR="$work/created" run sh "$here/component_entrypoint.sh" betterlite
expect_equal "created folder exit" 0 "$code"
[ -d "$work/created" ] || fail "missing folder was not created"

# A glob that matches no file must not fail the script.
mkdir "$work/no-db"
touch "$work/no-db/keep.txt"
PROMETHEUS_MULTIPROC_DIR="$work/no-db" run sh "$here/component_entrypoint.sh" betterlite
expect_equal "folder without db files exit" 0 "$code"
[ -e "$work/no-db/keep.txt" ] || fail "keep.txt was removed from a folder without db files"

touch "$work/cwd/keep.db"
cd "$work/cwd" || fail "cannot enter the scratch folder"
PROMETHEUS_MULTIPROC_DIR= run sh "$here/component_entrypoint.sh" betterlite
expect_equal "empty variable exit" 0 "$code"
expect_equal "empty variable leaves the folder" "keep.db" "$(ls -A "$work/cwd")"
rm "$work/cwd/keep.db"

run bash "$here/entrypoint.sh"
expect_equal "migrate output" "migrate${nl}Migration script ran successfully!" "$(cat "$work/out")"
expect_equal "migrate exit" 0 "$code"

run bash "$here/entrypoint.sh" --config "my config.yaml"
expect_equal "migrate extra args" "migrate${nl}--config${nl}my config.yaml${nl}Migration script ran successfully!" "$(cat "$work/out")"

for status in 1 2; do
    STUB_EXIT=$status run bash "$here/entrypoint.sh" --config c.yaml
    expect_equal "migrate exit $status" "$status" "$code"
    expect_equal "migrate $status output" "migrate${nl}--config${nl}c.yaml" "$(cat "$work/out")"
done

expected_ui="${nl}$work/cwd${nl}Admin UI - using default LiteLLM UI"
for variant in absent present; do
    rm -rf "$work/cwd"
    mkdir "$work/cwd"
    cd "$work/cwd" || fail "cannot enter the scratch folder"
    if [ "$variant" = present ]; then
        mkdir -p enterprise/enterprise_ui
        echo '{}' > enterprise/enterprise_ui/enterprise_colors.json
    fi
    # An empty PATH proves the script runs no external command; pwd and echo are builtins.
    PATH="$work/none" /bin/bash "$here/build_admin_ui.sh" > "$work/out" 2> "$work/err"
    expect_equal "admin ui $variant exit" 0 "$?"
    expect_equal "admin ui $variant output" "$expected_ui" "$(cat "$work/out")"
    expect_equal "admin ui $variant stderr" "" "$(cat "$work/err")"
done

echo ok
