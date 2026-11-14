#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use tracing::subscriber::set_default;
use uuid::Uuid;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::GuardrailSettings;
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::invocation::{GuardrailHookName, GuardrailInvocation};
use crate::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailSpanSink, GuardrailStatus,
};
use crate::purview_dlp_evaluation::content_check::{ContentCheck, ContentCheckError};
use crate::purview_dlp_evaluation::error_handling::POLICY_BLOCK_STATUS;
use crate::purview_dlp_evaluation::test_support::{
    Captured, GraphMock, Reply, guardrail_at, standard_replies, start,
};
use crate::purview_graph_client::wire::Activity;

const BLOCKED_BODY: &str = r#"{"policyActions":[{"@odata.type":"microsoft.graph.restrictAccessAction","restrictionAction":"block"}]}"#;
const PROCESS_PATH: &str = "/processContent";

fn request_data(call_id: Option<&str>) -> Object {
    let mut fields = Object::new();
    if let Some(given) = call_id {
        fields.insert("litellm_call_id".to_owned(), Json::Text(given.to_owned()));
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
        .result
        .unwrap();

    assert!(response.policy_actions.is_empty());
    assert_eq!(mock.count(PROCESS_PATH), 1);
}

#[rstest]
#[case::upload(Activity::UploadText, "uploadText")]
#[case::download(Activity::DownloadText, "downloadText")]
#[tokio::test]
async fn content_check_sends_the_text_and_activity_to_graph(
    #[case] activity: Activity,
    #[case] wire_name: &str,
) {
    let mock = start(standard_replies()).await;
    let guardrail = guardrail_at(&mock.base);
    let data = request_data(Some("call-1"));

    let _response = guardrail
        .check_content(&scan("secret text", &data, activity, true))
        .await
        .result
        .unwrap();

    let body = last_body(&mock, PROCESS_PATH);
    assert!(body.contains(r#""data":"secret text""#), "{body}");
    assert!(
        body.contains(&format!(r#""activity":"{wire_name}""#)),
        "{body}"
    );
}

#[tokio::test]
async fn content_check_computes_scopes_then_scans_the_given_user_with_the_scope_etag() {
    let mock = start(standard_replies()).await;
    let guardrail = guardrail_at(&mock.base);
    let data = request_data(None);
    let check = ContentCheck {
        user_id: "entra-user-7",
        ..scan("hello", &data, Activity::UploadText, true)
    };

    let _response = guardrail.check_content(&check).await.result.unwrap();

    let seen = mock.seen.lock().unwrap();
    let graph_calls: Vec<&str> = seen
        .iter()
        .map(|request| request.path.as_str())
        .filter(|path| path.starts_with("/users/"))
        .collect();
    assert_eq!(
        graph_calls,
        vec![
            "/users/entra-user-7/dataSecurityAndGovernance/protectionScopes/compute",
            "/users/entra-user-7/dataSecurityAndGovernance/processContent",
        ]
    );
    let process = seen
        .iter()
        .find(|request| request.path.ends_with(PROCESS_PATH))
        .unwrap();
    assert_eq!(
        process
            .headers
            .get("if-none-match")
            .map(|value| value.to_str().unwrap()),
        Some(r#""etag-1""#)
    );
}

#[tokio::test]
async fn content_check_uses_the_litellm_call_id_as_the_correlation_id() {
    let mock = start(standard_replies()).await;
    let guardrail = guardrail_at(&mock.base);
    let data = request_data(Some("call-1"));

    let _response = guardrail
        .check_content(&scan("hello", &data, Activity::UploadText, true))
        .await
        .result
        .unwrap();

    assert!(last_body(&mock, PROCESS_PATH).contains(r#""correlationId":"call-1""#));
}

#[tokio::test]
async fn content_check_generates_a_correlation_id_when_the_call_id_is_absent() {
    let mock = start(standard_replies()).await;
    let guardrail = guardrail_at(&mock.base);
    let data = request_data(None);

    let _response = guardrail
        .check_content(&scan("hello", &data, Activity::UploadText, true))
        .await
        .result
        .unwrap();

    let generated = Uuid::nil().to_string();
    assert!(last_body(&mock, PROCESS_PATH).contains(&format!(r#""correlationId":"{generated}""#)));
}

#[tokio::test]
async fn content_check_generates_a_correlation_id_when_the_call_id_is_empty() {
    let mock = start(standard_replies()).await;
    let guardrail = guardrail_at(&mock.base);
    let data = request_data(Some(""));

    let _response = guardrail
        .check_content(&scan("hello", &data, Activity::UploadText, true))
        .await
        .result
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
        .result
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
        .result
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
        .result
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

    let captured = Captured(Arc::new(Mutex::new(Vec::new())));
    let writer = captured.clone();
    let _guard = set_default(
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .with_writer(move || writer.clone())
            .finish(),
    );

    let response = guardrail
        .check_content(&scan("secret", &data, Activity::DownloadText, false))
        .await
        .result
        .unwrap();

    assert!(response.policy_actions.is_empty());
    assert!(response.protection_scope_state.is_none());
    let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains("WARN"), "{log}");
    assert!(
        log.contains("Purview DLP: API/network error in logging-only mode (not re-raised)"),
        "{log}"
    );
}

struct DiscardSink;

impl GuardrailSpanSink for DiscardSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn server_error() -> Reply {
    Reply {
        status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        headers: Vec::new(),
        body: String::new(),
    }
}

#[rstest]
#[case::pass(Reply::ok(r#"{"policyActions":[]}"#), true, GuardrailStatus::Success)]
#[case::block_when_blocking(Reply::ok(BLOCKED_BODY), true, GuardrailStatus::GuardrailIntervened)]
#[case::block_when_logging(Reply::ok(BLOCKED_BODY), false, GuardrailStatus::GuardrailIntervened)]
#[case::outage_when_blocking(server_error(), true, GuardrailStatus::GuardrailFailedToRespond)]
#[case::outage_when_logging(server_error(), false, GuardrailStatus::GuardrailFailedToRespond)]
#[tokio::test]
async fn content_check_records_the_guardrail_status(
    #[case] reply: Reply,
    #[case] block_on_violation: bool,
    #[case] status: GuardrailStatus,
) {
    let mock = start(replies_with(PROCESS_PATH, reply)).await;
    let guardrail = guardrail_at(&mock.base);
    let data = request_data(None);

    let outcome = guardrail
        .check_content(&scan(
            "secret",
            &data,
            Activity::UploadText,
            block_on_violation,
        ))
        .await;

    assert_eq!(outcome.record.guardrail_status, status);
    assert_eq!(
        outcome.record.guardrail_provider.as_deref(),
        Some("microsoft_purview")
    );
    assert_eq!(outcome.record.event_type, None);
}

#[tokio::test]
async fn content_check_records_the_process_content_answer() {
    let body = r#"{"policyActions":[{"@odata.type":"microsoft.graph.restrictAccessAction","action":"restrictAccess","restrictionAction":"block"}],"protectionScopeState":"modified"}"#;
    let mock = start(replies_with(PROCESS_PATH, Reply::ok(body))).await;
    let guardrail = guardrail_at(&mock.base);
    let data = request_data(None);

    let outcome = guardrail
        .check_content(&scan("secret", &data, Activity::UploadText, false))
        .await;

    let action = GuardrailValue::Object(BTreeMap::from([
        (
            "@odata.type".to_owned(),
            text("microsoft.graph.restrictAccessAction"),
        ),
        ("action".to_owned(), text("restrictAccess")),
        ("restrictionAction".to_owned(), text("block")),
    ]));
    assert_eq!(
        outcome.record.guardrail_response,
        GuardrailValue::Object(BTreeMap::from([
            (
                "policyActions".to_owned(),
                GuardrailValue::Array(vec![action])
            ),
            ("protectionScopeState".to_owned(), text("modified")),
        ]))
    );
}

#[rstest]
#[case::not_modified("notModified", Some("notModified"))]
#[case::modified("modified", Some("modified"))]
#[case::unknown("somethingNew", None)]
#[tokio::test]
async fn content_check_records_a_known_protection_scope_state(
    #[case] state: &str,
    #[case] recorded: Option<&str>,
) {
    let body = format!(r#"{{"policyActions":[],"protectionScopeState":"{state}"}}"#);
    let mock = start(replies_with(PROCESS_PATH, Reply::ok(&body))).await;
    let guardrail = guardrail_at(&mock.base);
    let data = request_data(None);

    let outcome = guardrail
        .check_content(&scan("hello", &data, Activity::UploadText, true))
        .await;

    let mut expected = BTreeMap::from([(
        "policyActions".to_owned(),
        GuardrailValue::Array(Vec::new()),
    )]);
    if let Some(scope_state) = recorded {
        expected.insert("protectionScopeState".to_owned(), text(scope_state));
    }
    assert_eq!(
        outcome.record.guardrail_response,
        GuardrailValue::Object(expected)
    );
}

#[tokio::test]
async fn content_check_records_an_empty_answer_when_graph_fails() {
    let mock = start(replies_with(PROCESS_PATH, server_error())).await;
    let guardrail = guardrail_at(&mock.base);
    let data = request_data(None);

    let outcome = guardrail
        .check_content(&scan("hello", &data, Activity::UploadText, false))
        .await;

    assert_eq!(
        outcome.record.guardrail_response,
        GuardrailValue::Object(BTreeMap::new())
    );
}

#[tokio::test]
async fn content_check_records_the_timings_from_the_guardrail_clock() {
    let mock = start(standard_replies()).await;
    let guardrail = guardrail_at(&mock.base);
    let data = request_data(None);
    let now = (guardrail.base.clock)()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();

    let outcome = guardrail
        .check_content(&scan("hello", &data, Activity::UploadText, true))
        .await;

    assert_eq!(outcome.record.start_time, Some(now));
    assert_eq!(outcome.record.end_time, Some(now));
    assert_eq!(outcome.record.duration, Some(0.0_f64));
}

fn records(data: &BTreeMap<String, GuardrailValue>) -> Vec<GuardrailValue> {
    let Some(GuardrailValue::Object(metadata)) = data.get("metadata") else {
        panic!("no metadata bucket in {data:?}");
    };
    let Some(GuardrailValue::Array(entries)) =
        metadata.get("standard_logging_guardrail_information")
    else {
        panic!("no guardrail records in {metadata:?}");
    };
    entries.clone()
}

fn record_field(entry: &GuardrailValue, key: &str) -> GuardrailValue {
    let GuardrailValue::Object(fields) = entry else {
        panic!("record is not an object: {entry:?}");
    };
    fields.get(key).cloned().unwrap_or(GuardrailValue::Null)
}

#[rstest]
#[case::pass(Reply::ok(r#"{"policyActions":[]}"#), "success", None)]
#[case::block(Reply::ok(BLOCKED_BODY), "guardrail_intervened", Some(400))]
#[case::outage(server_error(), "guardrail_failed_to_respond", Some(500))]
#[tokio::test]
async fn write_record_appends_the_record_before_returning_the_result(
    #[case] reply: Reply,
    #[case] status: &str,
    #[case] error_status: Option<u16>,
) {
    let mock = start(replies_with(PROCESS_PATH, reply)).await;
    let base = GuardrailBase::new(GuardrailSettings::default(), false).unwrap();
    let data = request_data(None);
    let mut request = BTreeMap::new();
    let mut invocation = GuardrailInvocation::begin(
        GuardrailHookName::StreamingIterator,
        &request,
        None,
        UNIX_EPOCH,
    )
    .unwrap();
    let mut context = GuardrailRecordContext {
        base: &base,
        request_data: &mut request,
        log_target: None,
        sink: &DiscardSink,
    };

    let result = guardrail_at(&mock.base)
        .check_content(&scan("secret", &data, Activity::UploadText, true))
        .await
        .write_record(&mut invocation, &mut context);

    let returned_status = match result {
        Ok(_response) => None,
        Err(ContentCheckError::Http(error)) => Some(error.status),
        Err(ContentCheckError::Record { source }) => panic!("record failed: {source}"),
    };
    assert_eq!(returned_status, error_status);
    assert!(invocation.self_recorded);
    let [entry]: [GuardrailValue; 1] = records(&request).try_into().unwrap();
    assert_eq!(record_field(&entry, "guardrail_status"), text(status));
    assert_eq!(
        record_field(&entry, "guardrail_provider"),
        text("microsoft_purview")
    );
}
