#![cfg(test)]

use super::Selection;

#[test]
fn an_empty_selection_runs_every_check() {
    assert!(Selection::default().includes("mutants"));
}

#[test]
fn skip_leaves_a_check_out_of_a_full_run() {
    let selection = Selection {
        only: Vec::new(),
        skip: vec!["mutants".to_owned(), "coverage".to_owned()],
    };
    assert!(selection.includes("tests"));
    assert!(!selection.includes("mutants"));
    assert!(!selection.includes("coverage"));
}

#[test]
fn skip_wins_over_only() {
    let selection = Selection {
        only: vec!["fmt".to_owned(), "mutants".to_owned()],
        skip: vec!["mutants".to_owned()],
    };
    assert!(selection.includes("fmt"));
    assert!(!selection.includes("mutants"));
    assert!(!selection.includes("clippy"));
}
