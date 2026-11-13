use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::logging_pre_post::classifier::is_classifier_call;

const ORIGIN: &str = r#"{"internal_call_origin":"autorouter_classifier"}"#;

#[rstest]
#[case::metadata("completion", &format!(r#"{{"metadata":{ORIGIN}}}"#), true)]
#[case::litellm_metadata("responses", &format!(r#"{{"litellm_metadata":{ORIGIN}}}"#), true)]
#[case::async_call_type("aresponses", &format!(r#"{{"metadata":{ORIGIN}}}"#), true)]
#[case::other_call_type("embedding", &format!(r#"{{"metadata":{ORIGIN}}}"#), false)]
#[case::other_origin(
    "completion",
    r#"{"metadata":{"internal_call_origin":"other"}}"#,
    false
)]
#[case::no_origin("completion", r#"{"metadata":{"user":"u1"}}"#, false)]
#[case::metadata_not_an_object("completion", r#"{"metadata":"autorouter_classifier"}"#, false)]
#[case::no_metadata("completion", "{}", false)]
fn logging_pre_post_behavior_tests_behavior(
    #[case] call_type: &str,
    #[case] params_json: &str,
    #[case] expected: bool,
) {
    let params: IndexMap<String, Json> =
        serde_json::from_str(params_json).expect("the fixture is valid");
    assert_eq!(is_classifier_call(call_type, &params), expected);
}
