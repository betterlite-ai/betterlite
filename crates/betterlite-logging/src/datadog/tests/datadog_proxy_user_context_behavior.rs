use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::datadog::apm::NullTracer;
use crate::datadog::proxy_failure::async_post_call_failure_hook;

use super::support::{
    Intake, build_logger, exception, exception_logging, field, has_key, json, last_queued, message,
    normalizer,
};

async fn logged_proxy_failure(caller_metadata: IndexMap<String, Json>) -> Json {
    let (logger, _) = build_logger(&[], Intake::Accepted);
    let patterns = normalizer();
    let facts = exception(
        r#"{"class_name": "AuthenticationError", "code": "401", "message": "invalid key"}"#,
    );
    let context = exception_logging(Some(&facts), &patterns);
    async_post_call_failure_hook(&logger, &NullTracer, &context, None, caller_metadata).await;
    message(&last_queued(&logger))
}

#[tokio::test]
async fn datadog_proxy_user_context_behavior() {
    let mut caller_metadata = IndexMap::new();
    caller_metadata.insert("user_api_key_hash".to_owned(), json("\"hash-1\""));
    caller_metadata.insert("user_api_key_alias".to_owned(), json("\"alias-1\""));
    caller_metadata.insert("team_id".to_owned(), json("\"team-1\""));
    caller_metadata.insert("user_api_key_user_id".to_owned(), json("\"user-1\""));
    caller_metadata.insert(
        "requester_metadata".to_owned(),
        json(r#"{"nested": [1, true, null]}"#),
    );

    // the message carries the caller map under LiteLLM's field name, unchanged
    let with_caller = logged_proxy_failure(caller_metadata.clone()).await;
    assert!(!has_key(&with_caller, "caller_metadata"));
    assert_eq!(
        field(&with_caller, "user_api_key_dict"),
        &Json::Object(caller_metadata)
    );
    assert_eq!(
        field(
            field(&with_caller, "user_api_key_dict"),
            "requester_metadata"
        ),
        &json(r#"{"nested": [1, true, null]}"#),
    );

    // an empty caller map stays an empty object, not null
    let without_caller = logged_proxy_failure(IndexMap::new()).await;
    assert_eq!(field(&without_caller, "user_api_key_dict"), &json("{}"));
}
