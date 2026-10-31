#![cfg(test)]

use crate::builtins::expect_budget;
use crate::builtins::file_size::size_problem;
use crate::builtins::globs::glob_set;
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
