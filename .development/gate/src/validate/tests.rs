#![cfg(test)]

use std::collections::BTreeMap;
use std::path::Path;

use crate::ids::{check_id_problem, task_id_problem};
use crate::spec::agents::Agents;
use crate::spec::backlog::{Backlog, BacklogEntry, Status};
use crate::spec::catalog::Catalog;
use crate::spec::task::Task;
use crate::validate::agents::runner_problems;
use crate::validate::command_args::{argument_problems, filtered_runs};
use crate::validate::needs::{self, needed_paths};
use crate::validate::task::{banned_tooling, crate_root_problems, scope_root_problems};
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
        "cd .development/templates && cmp -s \"$file\" \"../../$file\"",
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
        (
            "grep -q \"BATCH_SIZE.*10\" ../betterlite-worktrees/litellm/litellm/constants.py",
            vec!["a LiteLLM checkout outside the repository"],
        ),
        (
            "sha256sum ../litellm/litellm/proxy/openapi_registry.json",
            vec!["a LiteLLM checkout outside the repository"],
        ),
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
fn todo_scopes_never_cover_an_existing_crate_root() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut task: Task =
        read_yaml(&root.join(".development/tasks/B860-postgres-http.yaml")).unwrap();
    let mut entry: BacklogEntry = serde_saphyr::from_str(
        "{id: B860, milestone: M0, title: Probe, status: todo, depends_on: [], file: null, covers: [], parent: null, split_from: null, resolution: null, superseded_by: null, blocked_reason: null, pr: null}",
    )
    .unwrap();
    for path in [
        "crates/betterlite-config/**",
        "crates/betterlite-config/src/**",
        "crates/betterlite-config/src/lib.rs",
        "crates/betterlite-core/src/{lib,dispatch}.rs",
        "crates/betterlite/src/**",
    ] {
        task.scope.paths = vec![path.to_owned()];
        let problems = crate_root_problems(&root, &entry, &task);
        assert_eq!(problems.len(), 1, "{path}: {problems:?}");
    }
    task.scope.paths = [
        "crates/betterlite-config/src/sections/tls.rs",
        "crates/betterlite-config/src/sections/tls/**",
        "crates/betterlite-new/**",
        "crates/betterlite-config/Cargo.toml",
        "crates/betterlite/src/main.rs",
        "tools/oracle/src/lib.rs",
    ]
    .map(str::to_owned)
    .to_vec();
    assert_eq!(
        crate_root_problems(&root, &entry, &task),
        Vec::<String>::new()
    );
    task.scope.paths = vec!["crates/betterlite-config/**".to_owned()];
    entry.status = Status::Done;
    assert_eq!(
        crate_root_problems(&root, &entry, &task),
        Vec::<String>::new()
    );
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
    let errors: Task =
        read_yaml(&root.join(".development/tasks/B5102-error-response-status.yaml")).unwrap();
    assert!(
        needed_paths(&errors)
            .contains(&("tools/oracle/cases/helpers/openai-errors".to_owned(), true))
    );
}

#[test]
fn only_an_open_dependency_covers_a_missing_case() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let catalog: Catalog = read_yaml(&root.join(".development/checks.yaml")).unwrap();
    let mut tasks = BTreeMap::new();
    for (id, path, cases) in [
        ("B998", "tools/oracle/cases/probe/**", "[]"),
        (
            "B999",
            "crates/probe/**",
            "[tools/oracle/cases/probe/missing.yaml]",
        ),
    ] {
        let text = format!(
            "{{id: {id}, title: Probe, description: Probe., scope: {{paths: [{path}], lint_tables: false}}, references: [], dod: [{{text: Done., proof: [scope]}}], checks: {{optional: [], commands: []}}, oracle_cases: {cases}, deviations: [], out_of_scope: [], pitfalls: [], review_level: standard, budget: {{max_changed_lines: 100, max_new_expects: 0}}}}"
        );
        tasks.insert(
            id.to_owned(),
            serde_saphyr::from_str::<Task>(&text).unwrap(),
        );
    }
    for (status, found) in [("done", 1), ("todo", 0)] {
        let mut entries = Vec::new();
        for (id, entry_status, depends_on) in [("B998", status, "[]"), ("B999", "todo", "[B998]")] {
            let text = format!(
                "{{id: {id}, milestone: M0, title: Probe, status: {entry_status}, depends_on: {depends_on}, file: null, covers: [], parent: null, split_from: null, resolution: null, superseded_by: null, blocked_reason: null, pr: null}}"
            );
            entries.push(serde_saphyr::from_str::<BacklogEntry>(&text).unwrap());
        }
        let backlog = Backlog { tasks: entries };
        let problems = needs::problems(&root, &catalog, &backlog, &tasks).unwrap();
        assert_eq!(problems.len(), found, "{status}: {problems:?}");
    }
}
const LIBRARY_RUNS: [(&str, &[&str], &[&str]); 3] = [
    (
        "cargo nextest run --locked -p betterlite-cli --no-tests=fail -E 'test(/^autoroute::config::tests::x/)'",
        &["betterlite-cli"],
        &["autoroute"],
    ),
    (
        "cargo nextest run --locked --package=betterlite-core -E 'test(=relay::tests::x) | test(/^pillar::/)'",
        &["betterlite-core"],
        &["pillar", "relay"],
    ),
    (
        "cargo nextest run --locked -p a -p b --filterset 'test(/^shared::/)'",
        &["a", "b"],
        &["shared"],
    ),
];

const UNFILTERED_RUNS: [&str; 7] = [
    "cargo nextest run --locked -p betterlite-cli --test cli -E 'test(/^login::/)'",
    "cargo nextest run --locked -p betterlite-cli -E 'binary(cli) & test(/^login::/)'",
    "cargo nextest run --locked -p betterlite-cli -E 'kind(test) & test(/^login::/)'",
    "cargo nextest run --locked -E 'test(/^login::/)'",
    "cargo nextest run --locked -p betterlite-cli -E 'test(/login::/)'",
    "cargo nextest run --locked -p betterlite-cli -E 'test(/^Login::/)'",
    "cargo test --locked -p betterlite-cli login::",
];

const MODULE_CASES: [(&str, &str, &str, &str, &str, &str); 8] = [
    (
        "probe_module",
        "crates/probe/**",
        "crates/probe/**",
        "todo",
        "todo",
        "neither its scope nor an open dependency's creates one",
    ),
    (
        "keyring",
        "crates/probe/**",
        "crates/probe/**",
        "todo",
        "todo",
        "",
    ),
    (
        "probe_module",
        "crates/probe/**",
        "crates/probe/**",
        "todo",
        "in-progress",
        "",
    ),
    (
        "probe_module",
        "crates/betterlite-cli/tests/probe.rs",
        "crates/probe/**",
        "todo",
        "todo",
        "",
    ),
    (
        "probe_module",
        "crates/betterlite-cli/src/probe_module.rs",
        "crates/probe/**",
        "todo",
        "todo",
        "no crate root declares",
    ),
    (
        "probe_module",
        "crates/betterlite-cli/src/{lib,probe_module}.rs",
        "crates/probe/**",
        "todo",
        "todo",
        "",
    ),
    (
        "probe_module",
        "crates/probe/**",
        "crates/betterlite-cli/src/probe_module.rs",
        "todo",
        "todo",
        "",
    ),
    (
        "probe_module",
        "crates/probe/**",
        "crates/betterlite-cli/src/probe_module.rs",
        "done",
        "todo",
        "neither its scope nor an open dependency's creates one",
    ),
];

#[test]
fn filtered_runs_name_the_library_modules_a_filter_starts_at() {
    for (run, packages, modules) in LIBRARY_RUNS {
        let found = filtered_runs(run);
        assert_eq!(found.len(), 1, "{run}");
        let first = found.first().unwrap();
        assert_eq!(first.packages, packages, "{run}");
        assert_eq!(first.modules.iter().collect::<Vec<_>>(), modules, "{run}");
    }
    for run in UNFILTERED_RUNS {
        assert!(filtered_runs(run).is_empty(), "{run}");
    }
    let joined =
        "cargo nextest run -p a -E 'test(/^one::/)' && cargo nextest run -p b -E 'test(/^two::/)'";
    assert_eq!(filtered_runs(joined).len(), 2);
}

#[test]
fn a_filter_needs_a_module_that_main_has_or_an_open_task_creates() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let catalog: Catalog = read_yaml(&root.join(".development/checks.yaml")).unwrap();
    assert!(root.join("crates/betterlite-cli/src/keyring.rs").is_file());
    for (module, scope, owner_scope, owner_status, status, expected) in MODULE_CASES {
        let mut tasks = BTreeMap::new();
        let mut entries = Vec::new();
        for (id, path, filter, entry_status, depends_on) in [
            ("B998", owner_scope, "other", owner_status, "[]"),
            ("B999", scope, module, status, "[B998]"),
        ] {
            let task = format!(
                "{{id: {id}, title: Probe, description: Probe., scope: {{paths: ['{path}'], lint_tables: false}}, references: [], dod: [{{text: Done., proof: [probe]}}], checks: {{optional: [], commands: [{{id: probe, why: Probe., run: \"cargo nextest run --locked -p betterlite-cli --no-tests=fail -E 'test(/^{filter}::tests::/)'\"}}]}}, oracle_cases: [], deviations: [], out_of_scope: [], pitfalls: [], review_level: standard, budget: {{max_changed_lines: 100, max_new_expects: 0}}}}"
            );
            tasks.insert(
                id.to_owned(),
                serde_saphyr::from_str::<Task>(&task).unwrap(),
            );
            let entry = format!(
                "{{id: {id}, milestone: M0, title: Probe, status: {entry_status}, depends_on: {depends_on}, file: null, covers: [], parent: null, split_from: null, resolution: null, superseded_by: null, blocked_reason: null, pr: null}}"
            );
            entries.push(serde_saphyr::from_str::<BacklogEntry>(&entry).unwrap());
        }
        let backlog = Backlog { tasks: entries };
        let problems: Vec<String> = needs::problems(&root, &catalog, &backlog, &tasks)
            .unwrap()
            .into_iter()
            .filter(|problem| problem.starts_with("B999"))
            .collect();
        let case = format!("{module} {scope} {owner_scope} {owner_status} {status}");
        assert_eq!(
            problems.len(),
            usize::from(!expected.is_empty()),
            "{case}: {problems:?}"
        );
        assert!(
            problems.iter().all(|problem| problem.contains(expected)),
            "{case}: {problems:?}"
        );
    }
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
