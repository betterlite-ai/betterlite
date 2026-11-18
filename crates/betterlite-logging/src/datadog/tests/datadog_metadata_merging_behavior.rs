use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::datadog::apm::NullTracer;
use crate::datadog::proxy_failure::async_post_call_failure_hook;
use crate::datadog::service_events::{async_service_failure_hook, async_service_success_hook};
use crate::logging_service_events::types::ServiceType;

use super::support::{
    Intake, build_logger, exception, exception_logging, field, json, last_queued, message,
    normalizer, service_payload, text,
};

// safe_dumps sorts object keys, so a test compares the key set, not the order
fn fields_of(value: &Json) -> Vec<&str> {
    let Json::Object(fields) = value else {
        panic!("the expected value is a JSON object");
    };
    let mut names: Vec<&str> = fields.keys().map(String::as_str).collect();
    names.sort_unstable();
    names
}

#[tokio::test]
async fn datadog_metadata_merging_behavior() {
    let (logger, _) = build_logger(&[], Intake::Accepted);
    let payload = service_payload(ServiceType::Redis, true, Some("timeout"), 0.25);
    let overrides: IndexMap<String, Json> = serde_json::from_str(
        r#"{"service": "redis-override", "gauge_value": 2.5, "duration": 9.5, "key": "cache"}"#,
    )
    .unwrap();
    async_service_failure_hook(&logger, &payload, Some(&overrides));

    // the metadata lands in the payload object: a shared key takes the metadata value, new keys join it
    let merged = message(&last_queued(&logger));
    assert_eq!(
        fields_of(&merged),
        [
            "call_type",
            "caller",
            "duration",
            "error",
            "event_metadata",
            "gauge_value",
            "is_error",
            "key",
            "service",
        ]
        .as_slice(),
    );
    assert_eq!(text(field(&merged, "service")), "redis-override");
    assert_eq!(field(&merged, "duration"), &json("9.5"));
    assert_eq!(field(&merged, "gauge_value"), &json("2.5"));
    assert_eq!(text(field(&merged, "key")), "cache");
    // the payload keeps its own event_metadata field; the merge never nests under it
    assert_eq!(field(&merged, "event_metadata"), &Json::Null);

    // no metadata leaves the payload object with the fields the model dumped
    async_service_failure_hook(&logger, &payload, None);
    let unmerged = message(&last_queued(&logger));
    assert_eq!(
        fields_of(&unmerged),
        [
            "call_type",
            "caller",
            "duration",
            "error",
            "event_metadata",
            "is_error",
            "service",
        ]
        .as_slice(),
    );

    // the success hook merges the metadata the same way
    let reset = service_payload(ServiceType::ResetBudgetJob, false, None, 0.5);
    let labels: IndexMap<String, Json> =
        serde_json::from_str(r#"{"gauge_labels": "zone:west"}"#).unwrap();
    async_service_success_hook(&logger, &reset, Some(&labels));
    let reset_body = message(&last_queued(&logger));
    assert_eq!(text(field(&reset_body, "gauge_labels")), "zone:west");

    // the proxy failure hook takes no event metadata, so its message holds only its own five fields
    let patterns = normalizer();
    let facts = exception(r#"{"class_name": "APIError", "code": "500", "message": "failed"}"#);
    let context = exception_logging(Some(&facts), &patterns);
    async_post_call_failure_hook(&logger, &NullTracer, &context, None, IndexMap::new()).await;
    let proxy_body = message(&last_queued(&logger));
    assert_eq!(
        fields_of(&proxy_body),
        [
            "error_class",
            "exception",
            "status_code",
            "traceback",
            "user_api_key_dict",
        ]
        .as_slice(),
    );
}
