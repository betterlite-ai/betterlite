use std::sync::Arc;

use super::support;
use crate::api::responses_ws_managed::process::process_response_create;

const SESSION_FRAME: &str =
    r#"{"type": "response.create", "response": {"generate": false, "model": "gpt-4o-mini"}}"#;

#[tokio::test]
async fn a_generate_false_frame_is_acknowledged_without_a_provider_call() {
    let shared = support::shared();
    let mut connection = support::connection(support::MODEL, support::sink(&shared));
    let quota = support::ScriptedQuota::new(&shared, None);
    connection.quota_callbacks = Some(vec![Arc::new(quota)]);
    let mut handler = support::handler(connection);
    let provider = support::ScriptedProvider::idle(&shared);

    let (outcome, logs) = support::capture(process_response_create(
        &mut handler,
        &provider,
        SESSION_FRAME,
    ))
    .await;
    outcome.unwrap();
    assert_eq!(
        support::frames(&shared),
        vec![
            support::warmup_ack("response.created", "in_progress", "gpt-4o-mini"),
            support::warmup_ack("response.completed", "completed", "gpt-4o-mini"),
        ]
    );
    assert!(support::calls(&shared).is_empty());
    assert!(support::quotas(&shared).is_empty());
    assert!(support::trace(&shared).is_empty());
    assert_eq!(support::history(&handler), support::json("{}"));
    assert!(logs.is_empty());
}
