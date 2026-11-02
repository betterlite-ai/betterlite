#![cfg(test)]

use super::*;
use crate::arguments::json::Object;

fn message(role: &str) -> Message {
    Message {
        role: role.to_owned(),
        content: None,
        name: None,
        provider_specific_fields: Object::new(),
    }
}

#[test]
fn system_placement_behavior_tests_behavior() {
    assert_non_adjacent_system_indices_form_separate_runs();
}

// Two system messages three slots apart belong to different runs; only index-adjacent system messages merge into one.
fn assert_non_adjacent_system_indices_form_separate_runs() {
    let messages = vec![
        message("system"),
        message("system"),
        message("user"),
        message("user"),
        message("system"),
    ];
    assert_eq!(system_runs(&messages), vec![vec![0, 1], vec![4]]);
}
