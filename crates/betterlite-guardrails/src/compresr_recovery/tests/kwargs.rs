use rstest::rstest;

use crate::compresr_recovery::kwargs::follow_up_kwargs;

use super::support::map;

const OWN: &str = "token:compresr";

fn trimmed(kwargs: &str, own_marker: Option<&str>) -> String {
    let sanitized = follow_up_kwargs(&map(kwargs), own_marker).unwrap();
    serde_json::to_string(&sanitized).unwrap()
}

#[rstest]
#[case::own_marker_survives(
    r#"{"metadata":{"_pre_call_executed_guardrails":["token:other","token:compresr","token:third"],"user":"u"}}"#,
    Some(OWN),
    r#"{"metadata":{"_pre_call_executed_guardrails":["token:compresr"],"user":"u"}}"#
)]
#[case::no_own_marker_drops_the_key(
    r#"{"metadata":{"user":"u","_pre_call_executed_guardrails":["token:other"]}}"#,
    Some(OWN),
    r#"{"metadata":{"user":"u"}}"#
)]
#[case::guardrail_without_a_name_keeps_nothing(
    r#"{"metadata":{"_pre_call_executed_guardrails":["token:compresr"]}}"#,
    None,
    r#"{"metadata":{}}"#
)]
#[case::repeated_own_marker_stays(
    r#"{"metadata":{"_pre_call_executed_guardrails":["token:compresr","token:other","token:compresr"]}}"#,
    Some(OWN),
    r#"{"metadata":{"_pre_call_executed_guardrails":["token:compresr","token:compresr"]}}"#
)]
#[case::non_text_entries_go(
    r#"{"metadata":{"_pre_call_executed_guardrails":[5,null,"token:compresr"]}}"#,
    Some(OWN),
    r#"{"metadata":{"_pre_call_executed_guardrails":["token:compresr"]}}"#
)]
#[case::litellm_metadata_too(
    r#"{"litellm_metadata":{"_pre_call_executed_guardrails":["token:other"],"k":1}}"#,
    Some(OWN),
    r#"{"litellm_metadata":{"k":1}}"#
)]
#[case::both_buckets(
    r#"{"metadata":{"_pre_call_executed_guardrails":["a"]},"litellm_metadata":{"_pre_call_executed_guardrails":["token:compresr","b"]}}"#,
    Some(OWN),
    r#"{"metadata":{},"litellm_metadata":{"_pre_call_executed_guardrails":["token:compresr"]}}"#
)]
#[case::metadata_not_a_mapping(
    r#"{"metadata":"text","litellm_metadata":null}"#,
    Some(OWN),
    r#"{"metadata":"text","litellm_metadata":null}"#
)]
#[case::marker_list_not_a_list(
    r#"{"metadata":{"_pre_call_executed_guardrails":"token:other"}}"#,
    Some(OWN),
    r#"{"metadata":{"_pre_call_executed_guardrails":"token:other"}}"#
)]
#[case::no_marker_list(
    r#"{"metadata":{"user":"u"}}"#,
    Some(OWN),
    r#"{"metadata":{"user":"u"}}"#
)]
fn pre_call_markers_are_trimmed_to_the_own_marker_marker_trim(
    #[case] kwargs: &str,
    #[case] own_marker: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(trimmed(kwargs, own_marker), expected);
}

#[test]
fn own_state_and_the_logging_object_are_dropped_marker_trim() {
    let kwargs = r#"{"_compresr_state":1,"keep":2,"litellm_logging_obj":"x","_compresr":3,"compresr_plain":4}"#;
    assert_eq!(
        trimmed(kwargs, Some(OWN)),
        r#"{"keep":2,"compresr_plain":4}"#
    );
}
