use std::sync::Arc;

use super::support;
use crate::api::responses_ws_managed::process::process_response_create;

const INVALID_JSON: &str = r#"{"type": "error", "error": {"type": "invalid_request_error", "message": "Invalid JSON in response.create event"}}"#;

#[tokio::test]
async fn invalid_json_answers_once_and_messages_of_another_type_are_ignored() {
    let shared = support::shared();
    let mut connection = support::connection(support::MODEL, support::sink(&shared));
    let quota = support::ScriptedQuota::new(&shared, None);
    connection.quota_callbacks = Some(vec![Arc::new(quota)]);
    let mut handler = support::handler(connection);
    let provider = support::ScriptedProvider::idle(&shared);

    let ping = r#"{"type": "ping"}"#;
    let (ignored, _first_logs) =
        support::capture(process_response_create(&mut handler, &provider, ping)).await;
    ignored.unwrap();
    assert!(support::frames(&shared).is_empty());

    let (broken, logs) =
        support::capture(process_response_create(&mut handler, &provider, "not-json")).await;
    broken.unwrap();
    assert_eq!(support::frames(&shared), vec![support::json(INVALID_JSON)]);

    let numeric = r#"{"type": 7}"#;
    let (later, _later_logs) =
        support::capture(process_response_create(&mut handler, &provider, numeric)).await;
    later.unwrap();
    assert_eq!(support::frames(&shared).len(), 1);
    assert!(support::calls(&shared).is_empty());
    assert!(support::quotas(&shared).is_empty());
    assert!(support::trace(&shared).is_empty());
    assert_eq!(support::history(&handler), support::json("{}"));
    assert!(logs.is_empty());
}
