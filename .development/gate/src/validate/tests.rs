#![cfg(test)]

use std::path::Path;

use crate::ids::{check_id_problem, task_id_problem};
use crate::validate::validate;

#[test]
fn the_committed_development_folder_is_valid() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let problems = validate(&root).unwrap();
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn task_ids_are_b_and_three_digits() {
    assert!(task_id_problem("B004").is_none());
    for bad in ["B04", "C004", "B0045", "B00x"] {
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
