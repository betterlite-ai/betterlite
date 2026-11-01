#![cfg(test)]

use std::path::Path;

use crate::ids::{check_id_problem, task_id_problem};
use crate::spec::agents::Agents;
use crate::spec::catalog::Catalog;
use crate::spec::task::Task;
use crate::validate::agents::runner_problems;
use crate::validate::command_args::argument_problems;
use crate::validate::needs::needed_paths;
use crate::validate::task::{banned_tooling, scope_root_problems};
use crate::validate::validate;
use crate::yaml::read_yaml;

#[test]
fn the_committed_development_folder_is_valid() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let problems = validate(&root).unwrap();
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn task_ids_have_one_spelling_per_number() {
    for good in ["B004", "B999", "B1000", "B123456"] {
        assert!(task_id_problem(good).is_none(), "{good}");
    }
    for bad in ["B04", "C004", "B0045", "B00x", "B1234567", "B"] {
        assert!(task_id_problem(bad).is_some(), "{bad}");
    }
}

#[test]
fn check_ids_are_lowercase_kebab_case() {
    for good in ["oracle", "lint-table", "b000-version"] {
        assert!(check_id_problem(good).is_none(), "{good}");
    }
    for bad in ["Lint", "9lives", "", "snake_case"] {
        assert!(check_id_problem(bad).is_some(), "{bad}");
    }
}

#[test]
fn check_commands_run_python_only_as_the_pinned_litellm() {
    for good in [
        "jq -e '.[0].provider == \"MINIMAX\"' crates/betterlite-server/assets/a.json",
        "! uv run --locked --project tools/oracle/litellm python -c 'import litellm_enterprise'",
        "uv run --locked --project tools/oracle/litellm basedpyright tools/oracle/helpers",
        "awk '/^\\[/{t=1}' Cargo.toml",
        "f=$(mktemp) && cargo run -p betterlite-oracle > \"$f\"",
    ] {
        assert!(banned_tooling(good).is_empty(), "{good}");
    }
    for (bad, expected) in [
        ("python3 << 'CHECK'\nimport json\nCHECK", vec!["python3"]),
        (
            "uv run --with pyyaml python /tmp/check.py",
            vec!["python", "uv run --with", "/tmp"],
        ),
        (
            "rustc /tmp/check.rs -o /tmp/check && /tmp/check",
            vec!["rustc", "/tmp"],
        ),
        (
            "cd /home/user/litellm && grep -q x y",
            vec!["a home directory path"],
        ),
        ("cat > /tmp/a.json", vec!["/tmp"]),
    ] {
        assert_eq!(banned_tooling(bad), expected, "{bad}");
    }
}

#[test]
fn check_commands_pass_nextest_and_grep_arguments_they_accept() {
    for good in [
        "cargo nextest run --locked -p a --no-tests=fail -E 'test(/^x::/) | test(y)' -- --exact x::y",
        "INSTA_UPDATE=no cargo nextest run -p a -- --include-ignored --nocapture name",
        "cargo nextest run -p a && cargo run -p b -- --test-threads=1",
        "cargo nextest run -p a\ncargo run -p b -- --flag",
        "grep -qF -- '--include-team-keys' a.py && grep -qe '-x' a.py",
        "grep -q --include='*.rs' -e '--config' src && jq -e '.a | .b' f",
        "! grep -rqE 'x|-y' src",
    ] {
        assert!(argument_problems(good).is_empty(), "{good}");
    }
    for (bad, expected) in [
        (
            "cargo nextest run -p a -E 'test(/^x::/)' -- --test-threads=1",
            vec![
                "passes --test-threads=1 after --; cargo nextest run takes only test name filters and --exact, --skip, --ignored, --include-ignored, --nocapture there",
            ],
        ),
        (
            "X=1 cargo nextest run -p a --exact --skip=y",
            vec![
                "puts --exact before --; cargo nextest run takes it only after --",
                "puts --skip=y before --; cargo nextest run takes it only after --",
            ],
        ),
        (
            "grep -qF '@x' a.py && grep -qF \"--include-team-keys\" a.py",
            vec![
                "gives grep the pattern --include-team-keys before --, so grep reads it as an option; put -- before the pattern",
            ],
        ),
    ] {
        assert_eq!(argument_problems(bad), expected, "{bad}");
    }
}

#[test]
fn check_commands_pass_filtersets_nextest_can_parse() {
    let good = "cargo nextest run -p a --filterset='test(/^a::/) - test(/replays_the_/)'";
    assert!(argument_problems(good).is_empty(), "{good}");
    assert_eq!(
        argument_problems(
            "cargo nextest run -p a -E 'test(/^a::)' && cargo nextest run -p a -E='test(/^a(?!b)/)'"
        ),
        vec![
            "passes the filterset test(/^a::) that cargo nextest can't parse (expected close regex)",
            "passes the filterset test(/^a(?!b)/) that cargo nextest can't parse (invalid regex: look-around, including look-ahead and look-behind, is not supported)",
        ]
    );
}

#[test]
fn scope_paths_stay_under_the_scope_roots_or_existing_root_entries() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut catalog: Catalog = read_yaml(&root.join(".development/checks.yaml")).unwrap();
    catalog.settings.scope_roots = vec!["crates".to_owned(), "tools".to_owned()];
    let mut task: Task =
        read_yaml(&root.join(".development/tasks/B860-postgres-http.yaml")).unwrap();
    let good = [
        "crates/betterlite-new/src/lib.rs",
        "tools/oracle/**",
        "Cargo.toml",
        ".development/x",
    ];
    let bad = [
        "litellm/policy_templates_backup.json",
        "policy_templates.json",
        "packaging/homebrew/lite.rb",
    ];
    task.scope.paths = good
        .iter()
        .chain(&bad)
        .map(|path| (*path).to_owned())
        .collect();
    let flagged = scope_root_problems(&root, &catalog, &task);
    assert_eq!(flagged.len(), bad.len(), "{flagged:?}");
    for (problem, path) in flagged.iter().zip(bad) {
        assert!(
            problem.contains(&format!("scope path {path} is outside")),
            "{problem}"
        );
    }
}

#[test]
fn needed_paths_come_from_oracle_cases_and_test_flags() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixture: Task =
        read_yaml(&root.join(".development/tasks/B860-postgres-http.yaml")).unwrap();
    let needed = needed_paths(&fixture);
    assert!(needed.contains(&(
        "tools/oracle/fuzz/fuzz_targets/http_case.rs".to_owned(),
        false
    )));
    let passthrough: Task =
        read_yaml(&root.join(".development/tasks/B3600-passthrough-wire-helpers.yaml")).unwrap();
    assert!(needed_paths(&passthrough).contains(&(
        "tools/oracle/cases/passthrough/endpoint-type-vertex.yaml".to_owned(),
        false
    )));
}

#[test]
fn only_resume_arguments_name_the_session() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut agents: Agents = read_yaml(&folder.join("agents.yaml")).unwrap();
    let runner = &mut agents.runners[0];
    runner.args.push("{session}".to_owned());
    runner.resume = Some(vec!["--resume".to_owned()]);
    assert_eq!(
        runner_problems(runner),
        vec![
            format!(
                "agents.yaml: {} uses {{session}} in args; only resume has a session",
                runner.id
            ),
            format!(
                "agents.yaml: {} has resume arguments without {{session}}",
                runner.id
            ),
        ]
    );
}
