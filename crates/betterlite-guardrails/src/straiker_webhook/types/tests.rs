#![cfg(test)]

use rstest::rstest;

use super::{
    StraikerWebhookContent, StraikerWebhookContext, StraikerWebhookIdentity, StraikerWebhookUsage,
};

enum Record {
    Content,
    Context,
    Identity,
    Usage,
}

fn round_trip(record: &Record, source: &str) -> Result<String, serde_json::Error> {
    match record {
        Record::Content => {
            serde_json::to_string(&serde_json::from_str::<StraikerWebhookContent>(source)?)
        }
        Record::Context => {
            serde_json::to_string(&serde_json::from_str::<StraikerWebhookContext>(source)?)
        }
        Record::Identity => {
            serde_json::to_string(&serde_json::from_str::<StraikerWebhookIdentity>(source)?)
        }
        Record::Usage => {
            serde_json::to_string(&serde_json::from_str::<StraikerWebhookUsage>(source)?)
        }
    }
}

#[rstest]
#[case::content_defaults(Record::Content, "{}", Some(r#"{"texts":[],"images":[]}"#))]
#[case::content_drops_null_fields(
    Record::Content,
    r#"{"texts":["a"],"images":[],"structured_messages":null,"tools":[{"type":"function"}],"finish_reason":"stop"}"#,
    Some(r#"{"texts":["a"],"images":[],"tools":[{"type":"function"}],"finish_reason":"stop"}"#)
)]
#[case::context_requires_a_call_surface(Record::Context, "{}", None)]
#[case::context_omits_unset_fields(
    Record::Context,
    r#"{"call_surface":"completion","mode":null,"model_provider":"openai"}"#,
    Some(r#"{"call_surface":"completion","model_provider":"openai"}"#)
)]
#[case::context_keeps_a_mode_list(
    Record::Context,
    r#"{"call_surface":"completion","mode":["pre_call"]}"#,
    Some(r#"{"call_surface":"completion","mode":["pre_call"]}"#)
)]
#[case::identity_omits_absent_fields(
    Record::Identity,
    r#"{"litellm_key":"alias","litellm_team":null}"#,
    Some(r#"{"litellm_key":"alias"}"#)
)]
#[case::usage_omits_absent_counts(
    Record::Usage,
    r#"{"input_tokens":3,"output_tokens":null}"#,
    Some(r#"{"input_tokens":3}"#)
)]
fn straiker_webhook_context_tests_behavior(
    #[case] record: Record,
    #[case] source: &str,
    #[case] expected: Option<&str>,
) {
    let actual = round_trip(&record, source);
    match expected {
        Some(serialized) => assert_eq!(actual.unwrap(), serialized),
        None => {
            actual.unwrap_err();
        }
    }
}
