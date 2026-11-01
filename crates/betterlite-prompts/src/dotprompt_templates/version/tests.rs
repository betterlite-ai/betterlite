#![cfg(test)]

use super::super::PromptManager;
use super::super::template::PromptTemplate;
use super::strip_version_suffix;
use crate::arguments::json::Object;
use rstest::rstest;

#[rstest]
#[case::rightmost_v_suffix_with_digits("prompt.v1", Some("prompt"))]
#[case::only_the_rightmost_dot_v_is_stripped("prompt.v1.v2", Some("prompt.v1"))]
#[case::no_dot_v_separator("promptv1", None)]
#[case::version_is_not_digits("prompt.vabc", None)]
#[case::base_is_empty(".v1", None)]
#[case::version_is_empty("prompt.v", None)]
fn strip_version_suffix_matches_the_dot_v_rule(
    #[case] prompt_id: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(strip_version_suffix(prompt_id), expected);
}

fn manager_with(entries: &[(&str, &str)]) -> PromptManager {
    let mut prompts = indexmap::IndexMap::new();
    for (prompt_id, content) in entries {
        prompts.insert(
            (*prompt_id).to_owned(),
            PromptTemplate::new((*content).to_owned(), Object::new(), None).unwrap(),
        );
    }
    PromptManager {
        prompt_directory: None,
        prompts,
    }
}

#[rstest]
fn get_prompt_prefers_the_versioned_id_when_present() {
    let manager = manager_with(&[("greeting", "base"), ("greeting.v2", "versioned")]);
    assert_eq!(
        manager.get_prompt("greeting", Some(2)).unwrap().content,
        "versioned"
    );
}

#[rstest]
fn get_prompt_falls_back_to_a_direct_match_when_no_version_is_stored() {
    let manager = manager_with(&[("greeting", "base")]);
    assert_eq!(
        manager.get_prompt("greeting", Some(2)).unwrap().content,
        "base"
    );
}

#[rstest]
fn get_prompt_falls_back_to_base_id_for_versioned_prompt_id() {
    let manager = manager_with(&[("greeting", "base")]);
    assert_eq!(
        manager.get_prompt("greeting.v3", None).unwrap().content,
        "base"
    );
}

#[rstest]
fn get_prompt_returns_none_when_nothing_matches() {
    let manager = manager_with(&[("greeting", "base")]);
    assert!(manager.get_prompt("missing", None).is_none());
}
