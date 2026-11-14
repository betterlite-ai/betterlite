use rstest::rstest;

use super::support::{assert_json, data, record};
use crate::guardrail_metadata::append_guardrail_response;

const RECORD: &str = r#"{"guardrail_name":"model-armor","guardrail_provider":"model_armor","guardrail_mode":null,"guardrail_response":{"verdict":"allow"},"guardrail_status":"success","start_time":1.5,"end_time":null,"duration":null,"masked_entity_count":null}"#;

#[rstest]
#[case::no_logging_object(None, None)]
#[case::no_standard_logging_object(Some(r#"{"keep": 1}"#),Some(r#"{"keep": 1}"#.to_owned()))]
#[case::a_null_standard_logging_object_is_left_alone(Some(r#"{"standard_logging_object": null}"#),Some(r#"{"standard_logging_object": null}"#.to_owned()))]
#[case::a_missing_information_list_is_created(Some(r#"{"standard_logging_object": {"model": "m"}}"#),Some(format!(r#"{{"standard_logging_object": {{"model": "m", "guardrail_information": [{RECORD}]}}}}"#)))]
#[case::a_null_information_list_becomes_a_list(Some(r#"{"standard_logging_object": {"guardrail_information": null}}"#),Some(format!(r#"{{"standard_logging_object": {{"guardrail_information": [{RECORD}]}}}}"#)))]
#[case::an_existing_list_is_appended_to(Some(r#"{"standard_logging_object": {"guardrail_information": [{"guardrail_name": "earlier"}]}}"#),Some(format!(r#"{{"standard_logging_object": {{"guardrail_information": [{{"guardrail_name": "earlier"}}, {RECORD}]}}}}"#)))]
#[case::an_existing_record_is_appended_again(Some(r#"{"standard_logging_object": {"guardrail_information": [{"guardrail_name": "model-armor", "guardrail_provider": "model_armor", "guardrail_mode": null, "guardrail_response": {"verdict": "allow"}, "guardrail_status": "success", "start_time": 1.5, "end_time": null, "duration": null, "masked_entity_count": null}]}}"#),Some(format!(r#"{{"standard_logging_object": {{"guardrail_information": [{RECORD}, {RECORD}]}}}}"#)))]
#[case::other_call_details_are_kept(Some(r#"{"keep": 1, "standard_logging_object": {"model": "m"}}"#),Some(format!(r#"{{"keep": 1, "standard_logging_object": {{"model": "m", "guardrail_information": [{RECORD}]}}}}"#)))]
fn guardrail_metadata_cases(#[case] start: Option<&str>, #[case] expected: Option<String>) {
    let mut details = start.map(data);
    append_guardrail_response(details.as_mut(), &record()).unwrap();
    match expected {
        None => assert!(details.is_none()),
        Some(text) => assert_json(&details.unwrap(), &text),
    }
}
