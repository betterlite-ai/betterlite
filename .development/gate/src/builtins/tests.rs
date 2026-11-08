#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::builtins::case_ignores::pointer_problems;
use crate::builtins::expect_budget;
use crate::builtins::file_size::size_problem;
use crate::builtins::globs::glob_set;
use crate::builtins::module_tree::Walk;
use crate::spec::catalog::FileSize;

#[test]
fn a_single_star_stays_inside_one_directory() {
    let set = glob_set(&["crates/*".to_owned()]).unwrap();
    assert!(set.is_match("crates/Cargo.toml"));
    assert!(!set.is_match("crates/probe/src/lib.rs"));
}

#[test]
fn a_double_star_crosses_directories() {
    let set = glob_set(&["crates/probe/**".to_owned(), "Cargo.toml".to_owned()]).unwrap();
    assert!(set.is_match("crates/probe/src/lib.rs"));
    assert!(set.is_match("Cargo.toml"));
    assert!(!set.is_match("crates/other/src/lib.rs"));
    assert!(!set.is_match("crates/probe-extra/src/lib.rs"));
}

#[test]
fn a_task_file_pattern_matches_only_that_task() {
    let set = glob_set(&[".development/tasks/B208-*.yaml".to_owned()]).unwrap();
    assert!(set.is_match(".development/tasks/B208-http-proxies.yaml"));
    assert!(!set.is_match(".development/tasks/B2080-other.yaml"));
    assert!(!set.is_match(".development/backlog/B208.yaml"));
}

#[test]
fn the_expect_budget_counts_inner_and_outer_attributes() {
    let lines = [
        "#[expect(clippy::print_stdout, reason = \"the report is stdout\")]".to_owned(),
        "#![expect(dead_code, reason = \"wired in the next task\")]".to_owned(),
        "let budget = 1;".to_owned(),
    ];
    assert!(expect_budget::check(2, &lines).is_empty());
    assert_eq!(expect_budget::check(1, &lines).len(), 3);
}

#[test]
fn module_declaration_lines_do_not_count_toward_the_file_size() {
    let settings = FileSize {
        roots: vec!["crates".to_owned()],
        src_max_lines: 2,
        test_max_lines: 4,
    };
    let declarations = "pub mod alpha;\nmod beta;\npub mod gamma;\nuse std::fmt;\nfn run() {}\n";
    assert_eq!(
        size_problem("crates/probe/src/sections.rs", declarations, &settings),
        None
    );
    let code = "use std::fmt;\nmod inline {}\nfn run() {}\n";
    assert_eq!(
        size_problem("crates/probe/src/lib.rs", code, &settings).as_deref(),
        Some("crates/probe/src/lib.rs has 3 lines, limit is 2; split it along a real boundary")
    );
    assert_eq!(
        size_problem("crates/probe/src/tests.rs", code, &settings),
        None
    );
}

fn write(root: &Path, path: &str, text: &str) {
    let file = root.join(path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, text).unwrap();
}

#[test]
fn a_tests_file_its_parent_never_declares_is_not_reached() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    write(root, "src/lib.rs", "mod redirect;\n");
    write(root, "src/redirect.rs", "fn follow() {}\n");
    write(root, "src/redirect/tests.rs", "#[test]\nfn follows() {}\n");
    let walk = Walk::from_targets(&[root.join("src/lib.rs").as_path()]).unwrap();
    assert!(walk.reaches(&root.join("src/redirect.rs")));
    assert!(!walk.reaches(&root.join("src/redirect/tests.rs")));
}

#[test]
fn path_attributes_includes_inline_modules_and_mod_files_are_followed() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    let lib = "mod alpha;\nmod beta;\n#[path = \"gamma_impl.rs\"]\nmod gamma;\nmod outer {\n    mod inner;\n}\ninclude!(\"spliced.rs\");\ninclude!(\"parts/declarations.rs\");\n";
    write(root, "src/lib.rs", lib);
    write(root, "src/alpha.rs", "#[cfg(test)]\nmod tests;\n");
    write(root, "src/alpha/tests.rs", "");
    write(root, "src/beta/mod.rs", "mod child;\n");
    write(root, "src/beta/child.rs", "");
    write(root, "src/gamma_impl.rs", "mod sibling;\n");
    write(root, "src/sibling.rs", "");
    write(root, "src/outer/inner.rs", "");
    write(root, "src/spliced.rs", "mod delta;\n");
    write(root, "src/delta.rs", "");
    write(
        root,
        "src/parts/declarations.rs",
        "#[path = \"piece.rs\"]\nmod piece;\n",
    );
    write(root, "src/parts/piece.rs", "");
    let walk = Walk::from_targets(&[root.join("src/lib.rs").as_path()]).unwrap();
    for path in [
        "src/alpha/tests.rs",
        "src/beta/child.rs",
        "src/gamma_impl.rs",
        "src/sibling.rs",
        "src/outer/inner.rs",
        "src/spliced.rs",
        "src/delta.rs",
        "src/parts/piece.rs",
    ] {
        assert!(walk.reaches(&root.join(path)), "{path} is not reached");
    }
}

#[test]
fn files_an_item_macro_may_declare_count_as_reached() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    write(
        root,
        "src/lib.rs",
        "mod helpers;\nmacro_rules! noop { () => {} }\n",
    );
    write(root, "src/helpers.rs", "helper_group! { \"one\" => one }\n");
    write(root, "src/helpers/one.rs", "");
    write(root, "src/stray.rs", "");
    let walk = Walk::from_targets(&[root.join("src/lib.rs").as_path()]).unwrap();
    assert!(walk.reaches(&root.join("src/helpers/one.rs")));
    assert!(!walk.reaches(&root.join("src/stray.rs")));
}

#[test]
fn helper_case_pointers_start_with_a_field_of_the_answer() {
    let case = "kind: helper\nname: probe\ncompare:\n  ignore:\n    - pointer: \"\"\n      why: all\n    - pointer: /value/creator\n      why: generated\n    - pointer: /creator\n      why: generated\n    - pointer: value/creator\n      why: generated\n";
    let problems = pointer_problems("tools/oracle/cases/probe.yaml", case).unwrap();
    assert_eq!(problems.len(), 2);
    assert!(problems[0].contains("\"/creator\""));
    assert!(problems[1].contains("\"value/creator\""));
}

#[test]
fn http_case_pointers_are_left_to_the_oracle() {
    let case = "# a comment\nname: probe\ncompare:\n  headers: []\n  ignore:\n    - pointer: /id\n      why: random\n";
    assert!(
        pointer_problems("tools/oracle/cases/probe.yaml", case)
            .unwrap()
            .is_empty()
    );
}
