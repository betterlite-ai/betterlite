use betterlite_models::decisions::Extra;
use betterlite_models::decisions::openai::{self, ChoiceValue};
use betterlite_models::decisions::systemone::{DecisionQuestion, NoulCriteria, NoulQuestion};
use betterlite_models::decisions::validators::{Checked, Unchecked, ValidationError};
use rstest::rstest;

#[rstest]
#[case::neither(r#"{"type":"noul"}"#, false)]
#[case::nulls(r#"{"type":"noul","instructions":null,"criteria":null}"#, false)]
#[case::instructions(r#"{"type":"noul","instructions":"Urgent?"}"#, true)]
#[case::criteria(r#"{"type":"noul","criteria":{"true":"now"}}"#, true)]
fn noul_needs_instructions_or_criteria(#[case] json: &str, #[case] accepted: bool) {
    assert_eq!(
        serde_json::from_str::<DecisionQuestion>(json).is_ok(),
        accepted
    );
}

#[test]
fn builds_noul_questions_through_the_check() {
    let bare = NoulQuestion {
        instructions: None,
        criteria: None,
        extra: Extra::new(),
    };
    assert_eq!(
        Checked::try_from(Unchecked(bare)).unwrap_err(),
        ValidationError::NoulPrompt
    );
    let asked = NoulQuestion {
        instructions: None,
        criteria: Some(NoulCriteria::new()),
        extra: Extra::new(),
    };
    assert_eq!(
        Checked::try_from(Unchecked(asked.clone()))
            .unwrap()
            .as_ref(),
        &asked
    );
}

#[rstest]
#[case::flag_and_text(r#"true,"true""#, false)]
#[case::false_and_text(r#""false",false"#, false)]
#[case::same_text(r#""a","a""#, false)]
#[case::both_flags("true,false", true)]
#[case::flag_texts(r#""true","false""#, true)]
fn choice_values_map_to_unique_keys(#[case] values: &str, #[case] accepted: bool) {
    let choices: Vec<String> = values
        .split(',')
        .map(|value| format!(r#"{{"value":{value}}}"#))
        .collect();
    let json = format!(
        r#"{{"type":"choice","instructions":"Pick","choices":[{}]}}"#,
        choices.join(",")
    );
    assert_eq!(
        serde_json::from_str::<openai::DecisionQuestion>(&json).is_ok(),
        accepted
    );
}

#[rstest]
#[case::true_flag(ChoiceValue::Flag(true), "true")]
#[case::false_flag(ChoiceValue::Flag(false), "false")]
#[case::text(ChoiceValue::Text("tech".to_owned()), "tech")]
fn choice_values_name_their_systemone_key(#[case] value: ChoiceValue, #[case] key: &str) {
    assert_eq!(value.systemone_key(), key);
}
