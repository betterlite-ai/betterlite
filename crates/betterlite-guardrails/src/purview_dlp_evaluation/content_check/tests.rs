#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_prompts::arguments::json::{Json, Object};
use uuid::Uuid;

use crate::purview_dlp_evaluation::content_check::ContentCheck;
use crate::purview_dlp_evaluation::error_handling::POLICY_BLOCK_STATUS;
use crate::purview_dlp_evaluation::test_support::{
    GraphMock, Reply, guardrail_at, standard_replies, start,
};
use crate::purview_graph_client::wire::Activity;

const BLOCKED_BODY: &str = r#"{"policyActions":[{"@odata.type":"microsoft.graph.restrictAccessAction","restrictionAction":"block"}]}"#;
const PROCESS_PATH: &str = "/processContent";

fn request_data(call_id: Option<&str>) -> Object {
    let mut fields = Object::new();
    if let Some(call_id) = call_id {
        fields.insert("litellm_call_id".to_owned(), Json::Text(call_id.to_owned()));
    }
    fields
}

fn scan<'request>(
    text: &'request str,
    data: &'request Object,
    activity: Activity,
    block_on_violation: bool,
) -> ContentCheck<'request> {
    ContentCheck {
        user_id: "user-1",
        text,
        activity,
        request_data: data,
        block_on_violation,
    }
}

fn last_body(mock: &GraphMock, suffix: &str) -> String {
    mock.seen
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|request| request.path.ends_with(suffix))
        .unwrap()
        .body
        .clone()
}

fn replies_with(suffix: &str, reply: Reply) -> Vec<(&'static str, Reply)> {
    let mut replies = standard_replies();
    let entry = replies
        .iter_mut()
        .find(|(path_suffix, _reply)| *path_suffix == suffix)
        .unwrap();
    entry.1 = reply;
    replies
}

#[tokio::test]
async fn content_check_returns_the_process_content_answer() {
    let mock = start(standard_replies()).await;
    let guardrail = guardrail_at(&mock.base);
    let data = request_data(Some("call-1"));

    let response = guardrail
        .check_content(&scan("hello", &data, Activity::UploadText, true))
        .await
        .unwrap();

    assert!(response.policy_actions.is_empty());
    assert_eq!(mock.count(PROCESS_PATH), 1);
}

#[tokio::test]
async fn content_check_sends_the_text_and_activity_to_graph() {
    let mock = start(standard_replies()).await;
    let guardrail = guardrail_at(&mock.base);
    let data = request_data(Some("call-1"));

    guardrail
        .check_content(&scan("secret text", &data, Activity::UploadText, true))
        .await
        .unwrap();

    let body = last_body(&mock, PROCESS_PATH);
    assert!(body.contains(r#""data":"secret text""#), "{body}");
    assert!(body.contains(r#""activity":"uploadText""#), "{body}");
}

#[tokio::test]
async fn content_check_uses_the_litellm_call_id_as_the_correlation_id() {
    let mock = start(standard_replies()).await;
    let guardrail = guardrail_at(&mock.base);
    let data = request_data(Some("call-1"));

    guardrail
        .check_content(&scan("hello", &data, Activity::UploadText, true))
        .await
        .unwrap();

    assert!(last_body(&mock, PROCESS_PATH).contains(r#""correlationId":"call-1""#));
}

#[tokio::test]
async fn content_check_generates_a_correlation_id_when_the_call_id_is_absent() {
    let mock = start(standard_replies()).await;
    let guardrail = guardrail_at(&mock.base);
    let data = request_data(None);

    guardrail
        .check_content(&scan("hello", &data, Activity::UploadText, true))
        .await
        .unwrap();

    let generated = Uuid::nil().to_string();
    assert!(last_body(&mock, PROCESS_PATH).contains(&format!(r#""correlationId":"{generated}""#)));
}

#[tokio::test]
async fn content_check_generates_a_correlation_id_when_the_call_id_is_empty() {
    let mock = start(standard_replies()).await;
    let guardrail = guardrail_at(&mock.base);
    let data = request_data(Some(""));

    guardrail
        .check_content(&scan("hello", &data, Activity::UploadText, true))
        .await
        .unwrap();

    let generated = Uuid::nil().to_string();
    assert!(last_body(&mock, PROCESS_PATH).contains(&format!(r#""correlationId":"{generated}""#)));
}

#[tokio::test]
async fn content_check_raises_a_policy_block_in_blocking_mode() {
    let mock = start(replies_with(PROCESS_PATH, Reply::ok(BLOCKED_BODY))).await;
    let guardrail = guardrail_at(&mock.base);
    let data = request_data(None);

    let error = guardrail
        .check_content(&scan("secret", &data, Activity::UploadText, true))
        .await
        .unwrap_err();

    assert_eq!(error.status, POLICY_BLOCK_STATUS);
    assert_eq!(
        error.detail.get("error"),
        Some(&GuardrailValue::Text(
            "Microsoft Purview DLP: Content blocked by policy".to_owned()
        ))
    );
    assert_eq!(
        error.detail.get("activity"),
        Some(&GuardrailValue::Text("uploadText".to_owned()))
    );
}

#[tokio::test]
async fn content_check_does_not_raise_a_policy_block_in_logging_only_mode() {
    let mock = start(replies_with(PROCESS_PATH, Reply::ok(BLOCKED_BODY))).await;
    let guardrail = guardrail_at(&mock.base);
    let data = request_data(None);

    let response = guardrail
        .check_content(&scan("secret", &data, Activity::DownloadText, false))
        .await
        .unwrap();

    assert_eq!(response.policy_actions.len(), 1);
}

#[tokio::test]
async fn content_check_propagates_a_rate_limit_in_blocking_mode() {
    let mock = start(replies_with(
        PROCESS_PATH,
        Reply {
            status: axum::http::StatusCode::TOO_MANY_REQUESTS,
            headers: vec![("retry-after", "17")],
            body: String::new(),
        },
    ))
    .await;
    let guardrail = guardrail_at(&mock.base);
    let data = request_data(None);

    let error = guardrail
        .check_content(&scan("secret", &data, Activity::UploadText, true))
        .await
        .unwrap_err();

    assert_eq!(error.status, 429);
    assert_eq!(error.retry_after.as_deref(), Some("17"));
}

#[tokio::test]
async fn content_check_swallows_an_upstream_failure_in_logging_only_mode() {
    let mock = start(replies_with(
        PROCESS_PATH,
        Reply {
            status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            headers: Vec::new(),
            body: String::new(),
        },
    ))
    .await;
    let guardrail = guardrail_at(&mock.base);
    let data = request_data(None);

    let response = guardrail
        .check_content(&scan("secret", &data, Activity::DownloadText, false))
        .await
        .unwrap();

    assert!(response.policy_actions.is_empty());
    assert!(response.protection_scope_state.is_none());
}
