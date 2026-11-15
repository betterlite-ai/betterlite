#![cfg(test)]

use super::character_count;
use crate::cost::completion::error::ChargeInputError;

#[test]
fn a_character_count_keeps_its_value_and_an_absent_one_stays_absent() {
    assert_eq!(character_count("prompt_characters", None).unwrap(), None);
    assert_eq!(
        character_count("prompt_characters", Some(7)).unwrap(),
        Some(7)
    );
}

#[test]
fn a_count_past_the_signed_range_names_its_field() {
    let error = character_count("completion_characters", Some(u64::MAX)).unwrap_err();
    assert!(matches!(
        error,
        ChargeInputError::CharacterCount {
            field: "completion_characters",
            ..
        }
    ));
}
