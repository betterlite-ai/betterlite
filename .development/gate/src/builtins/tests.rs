#![cfg(test)]

use crate::builtins::expect_budget;
use crate::builtins::globs::glob_set;

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
fn the_expect_budget_counts_inner_and_outer_attributes() {
    let lines = [
        "#[expect(clippy::print_stdout, reason = \"the report is stdout\")]".to_owned(),
        "#![expect(dead_code, reason = \"wired in the next task\")]".to_owned(),
        "let budget = 1;".to_owned(),
    ];
    assert!(expect_budget::check(2, &lines).is_empty());
    assert_eq!(expect_budget::check(1, &lines).len(), 3);
}
