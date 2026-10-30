#![cfg(test)]

use super::str_to_bool;

#[test]
fn str_to_bool_accepts_true_variants_case_insensitively() {
    assert!(str_to_bool("true"));
    assert!(str_to_bool("TRUE"));
    assert!(str_to_bool("1"));
    assert!(str_to_bool("yes"));
}

#[test]
fn str_to_bool_rejects_everything_else() {
    assert!(!str_to_bool("false"));
    assert!(!str_to_bool("0"));
    assert!(!str_to_bool("no"));
    assert!(!str_to_bool(""));
}
