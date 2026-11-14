#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, Method, Uri};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_http::proxy::base_builder;
use reqwest::StatusCode;
use rstest::rstest;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use uuid::Uuid;

use super::apply::UNAVAILABLE_MESSAGE;
use super::{Clock, HeadroomGuardrail};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::headroom_settings::{HeadroomInput, HeadroomSettings};

mod compress;

const FOUR: &str = r#"{"structured_messages":[{"role":"system","content":"sys"},{"role":"user","content":"long text"},{"role":"assistant","content":"answer"},{"role":"user","content":"now"}]}"#;
const THREE_USERS: &str = r#"{"structured_messages":[{"role":"user","content":"one"},{"role":"user","content":"two"},{"role":"user","content":"three"}]}"#;
const SINGLE_USER: &str = r#"{"structured_messages":[{"role":"user","content":"only"}]}"#;
const SERVICE_ONE: &str = r#"{"messages":[{"role":"user","content":"short"}]}"#;
const SERVICE_TWO: &str =
    r#"{"messages":[{"role":"user","content":"a"},{"role":"user","content":"b"}]}"#;
const SERVICE_HASHES: &str =
    r#"{"messages":[{"role":"user","content":"short"}],"ccr_hashes":["ABCDEF123456"]}"#;
const SENT_HASH: &str = "abcdef123456";

#[derive(Clone)]
struct Seen {
    method: String,
    path: String,
    authorization: Option<String>,
    content_type: Option<String>,
    body: String,
}

struct Mock {
    base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    _server: JoinSet<io::Result<()>>,
}

async fn mock(status: StatusCode, payload: String) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_by_handler = Arc::clone(&seen);
    let answer = move |method: Method, uri: Uri, headers: HeaderMap, bytes: Bytes| {
        let recorder = Arc::clone(&seen_by_handler);
        let reply = payload.clone();
        async move {
            let header = |name: &str| {
                headers
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned)
            };
            recorder.lock().unwrap().push(Seen {
                method: method.to_string(),
                path: uri.path().to_owned(),
                authorization: header("authorization"),
                content_type: header("content-type"),
                body: String::from_utf8(bytes.to_vec()).unwrap(),
            });
            (status, [("content-type", "application/json")], reply)
        }
    };
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, Router::new().fallback(answer)).into_future());
    Mock {
        base,
        seen,
        _server: server,
    }
}

fn call_count(server: &Mock) -> usize {
    server.seen.lock().unwrap().len()
}

fn only_call(server: &Mock) -> Seen {
    server
        .seen
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("the compression service was called")
}

fn default_input(base: &str) -> HeadroomInput {
    HeadroomInput {
        api_base: Some(base.to_owned()),
        api_key: Some(SecretString::from("sk-test")),
        ..HeadroomInput::default()
    }
}

fn fail_open_input(base: &str) -> HeadroomInput {
    HeadroomInput {
        unreachable_fallback: Some("fail_open".to_owned()),
        ..default_input(base)
    }
}

fn build(input: HeadroomInput, now: &Arc<Mutex<Duration>>) -> HeadroomGuardrail {
    let env: BTreeMap<String, String> = BTreeMap::new();
    let settings = HeadroomSettings::new(&input, &env).unwrap();
    let shared = Arc::clone(now);
    let clock: Clock = Box::new(move || *shared.lock().unwrap());
    HeadroomGuardrail::new(
        Some("headroom".to_owned()),
        settings,
        base_builder(),
        clock,
        Uuid::new_v4,
    )
    .unwrap()
}

fn guardrail(base: &str) -> HeadroomGuardrail {
    build(default_input(base), &Arc::new(Mutex::new(Duration::ZERO)))
}

fn clocked(input: HeadroomInput) -> (HeadroomGuardrail, Arc<Mutex<Duration>>) {
    let now = Arc::new(Mutex::new(Duration::ZERO));
    let built = build(input, &now);
    (built, now)
}

async fn apply(
    guardrail: &HeadroomGuardrail,
    given: GuardrailApiInputs,
    data: &mut BTreeMap<String, GuardrailValue>,
) -> Result<GuardrailApiInputs, GuardrailFailure> {
    guardrail
        .apply(given, data, GuardrailInputType::Request, None)
        .await
}

fn inputs(text: &str) -> GuardrailApiInputs {
    serde_json::from_str(text).unwrap()
}

fn data(pairs: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(pairs).unwrap()
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap()
}

fn logging_target(call_id: &str) -> GuardrailLogTarget {
    GuardrailLogTarget {
        litellm_params: None,
        model_call_params: Some(BTreeMap::from([(
            "litellm_call_id".to_owned(),
            GuardrailValue::Text(call_id.to_owned()),
        )])),
    }
}

fn metadata(data: &BTreeMap<String, GuardrailValue>) -> serde_json::Value {
    let encoded = serde_json::to_value(data).unwrap();
    let key = if encoded.get("litellm_metadata").is_some() {
        "litellm_metadata"
    } else {
        "metadata"
    };
    encoded.get(key).cloned().unwrap_or(serde_json::Value::Null)
}

fn records(data: &BTreeMap<String, GuardrailValue>) -> Vec<serde_json::Value> {
    metadata(data)
        .get("standard_logging_guardrail_information")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn first_record(data: &BTreeMap<String, GuardrailValue>) -> serde_json::Value {
    records(data).first().cloned().expect("a guardrail record")
}

fn applied_names(data: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
    metadata(data)
        .get("applied_guardrails")
        .and_then(serde_json::Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn record_response(data: &BTreeMap<String, GuardrailValue>) -> serde_json::Value {
    first_record(data)
        .get("guardrail_response")
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

fn record_status(data: &BTreeMap<String, GuardrailValue>) -> serde_json::Value {
    first_record(data)
        .get("guardrail_status")
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

fn structure(returned: &GuardrailApiInputs) -> serde_json::Value {
    serde_json::to_value(returned.structured_messages.as_ref().unwrap()).unwrap()
}

#[rstest]
#[case::lowercase(
    r#"{"proxy_server_request":{"headers":{"x-headroom-bypass":"true"}}}"#,
    true
)]
#[case::uppercase(
    r#"{"proxy_server_request":{"headers":{"x-headroom-bypass":"TRUE"}}}"#,
    true
)]
#[case::mixed(
    r#"{"proxy_server_request":{"headers":{"x-headroom-bypass":"True"}}}"#,
    true
)]
#[case::false_text(
    r#"{"proxy_server_request":{"headers":{"x-headroom-bypass":"false"}}}"#,
    false
)]
#[case::empty(
    r#"{"proxy_server_request":{"headers":{"x-headroom-bypass":""}}}"#,
    false
)]
#[case::other_text(
    r#"{"proxy_server_request":{"headers":{"x-headroom-bypass":"yes"}}}"#,
    false
)]
#[case::boolean_json(
    r#"{"proxy_server_request":{"headers":{"x-headroom-bypass":true}}}"#,
    false
)]
#[case::request_not_an_object(r#"{"proxy_server_request":"text"}"#, false)]
#[case::headers_not_an_object(r#"{"proxy_server_request":{"headers":"text"}}"#, false)]
#[case::missing(r#"{"model":"gpt-4o"}"#, false)]
#[tokio::test]
async fn the_bypass_header_skips_compression_only_for_true(
    #[case] given: &str,
    #[case] bypasses: bool,
) {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let original = inputs(FOUR);
    let mut request = data(given);
    let returned = apply(&guardrail, original.clone(), &mut request)
        .await
        .unwrap();
    assert_eq!(returned, original);
    if bypasses {
        assert_eq!(call_count(&server), 0);
        assert!(records(&request).is_empty());
    } else {
        assert_eq!(call_count(&server), 1);
    }
}

#[rstest]
#[case::enabled(r#"{"background":true}"#, true)]
#[case::disabled(r#"{"background":false}"#, false)]
#[case::string_value(r#"{"background":"true"}"#, false)]
#[case::missing(r#"{"model":"gpt-4o"}"#, false)]
#[tokio::test]
async fn background_requests_skip_compression(#[case] given: &str, #[case] skipped: bool) {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let original = inputs(FOUR);
    let mut request = data(given);
    let returned = apply(&guardrail, original.clone(), &mut request)
        .await
        .unwrap();
    assert_eq!(returned, original);
    if skipped {
        assert_eq!(call_count(&server), 0);
    } else {
        assert_eq!(call_count(&server), 1);
    }
}

#[tokio::test]
async fn response_inputs_are_left_unchanged() {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let original = inputs(FOUR);
    let mut request = data("{}");
    let returned = guardrail
        .apply(
            original.clone(),
            &mut request,
            GuardrailInputType::Response,
            None,
        )
        .await
        .unwrap();
    assert_eq!(returned, original);
    assert_eq!(call_count(&server), 0);
}

#[rstest]
#[case::absent(r#"{"model":"gpt-4o"}"#)]
#[case::empty_list(r#"{"structured_messages":[]}"#)]
#[tokio::test]
async fn requests_without_messages_are_left_unchanged(#[case] given: &str) {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let original = inputs(given);
    let mut request = data("{}");
    let returned = apply(&guardrail, original.clone(), &mut request)
        .await
        .unwrap();
    assert_eq!(returned, original);
    assert_eq!(call_count(&server), 0);
}

#[tokio::test]
async fn requests_with_nothing_compressible_are_left_unchanged() {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let original = inputs(SINGLE_USER);
    let mut request = data("{}");
    let returned = apply(&guardrail, original.clone(), &mut request)
        .await
        .unwrap();
    assert_eq!(returned, original);
    assert_eq!(call_count(&server), 0);
}

#[tokio::test]
async fn a_non_list_request_messages_field_contributes_no_ids() {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let mut request = data(r#"{"messages":{"role":"user"}}"#);
    let returned = apply(&guardrail, inputs(FOUR), &mut request).await.unwrap();
    assert_eq!(
        structure(&returned),
        json(
            r#"[{"role":"system","content":"sys"},{"role":"user","content":"short"},{"role":"assistant","content":"answer"},{"role":"user","content":"now"}]"#
        )
    );
    assert_eq!(call_count(&server), 1);
}

#[tokio::test]
async fn protected_rows_are_restored_in_their_positions() {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let mut request = data("{}");
    let returned = apply(&guardrail, inputs(FOUR), &mut request).await.unwrap();
    assert_eq!(
        structure(&returned),
        json(
            r#"[{"role":"system","content":"sys"},{"role":"user","content":"short"},{"role":"assistant","content":"answer"},{"role":"user","content":"now"}]"#
        )
    );
    let sent = json(&only_call(&server).body);
    assert_eq!(
        sent,
        json(r#"{"messages":[{"role":"user","content":"long text"}]}"#)
    );
}

#[tokio::test]
async fn untranslated_tool_results_are_protected_from_compression() {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let given = inputs(
        r#"{"structured_messages":[{"role":"user","content":"one"},{"role":"tool","tool_call_id":"t1","content":"retrieved secret"},{"role":"user","content":"final"}]}"#,
    );
    let mut request = data(
        r#"{"messages":[{"role":"assistant","content":"","tool_calls":[{"id":"t1","type":"function","function":{"name":"mcp__headroom__headroom_retrieve","arguments":"{}"}}]},{"role":"tool","tool_call_id":"t1","content":"retrieved secret"}]}"#,
    );
    let returned = apply(&guardrail, given, &mut request).await.unwrap();
    assert_eq!(
        structure(&returned),
        json(
            r#"[{"role":"user","content":"short"},{"role":"tool","tool_call_id":"t1","content":"retrieved secret"},{"role":"user","content":"final"}]"#
        )
    );
    let sent = json(&only_call(&server).body);
    assert_eq!(
        sent.get("messages")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1)
    );
}

#[tokio::test]
async fn call_id_comes_from_the_logging_object() {
    let server = mock(StatusCode::OK, SERVICE_HASHES.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let target = logging_target("call-1");
    let mut request = data(r#"{"litellm_call_id":"request-id"}"#);
    guardrail
        .apply(
            inputs(FOUR),
            &mut request,
            GuardrailInputType::Request,
            Some(&target),
        )
        .await
        .unwrap();
    assert_eq!(
        guardrail.has_issued_hashes("call-1"),
        Some([SENT_HASH.to_owned()].into_iter().collect())
    );
    assert_eq!(guardrail.has_issued_hashes("request-id"), None);
}

#[tokio::test]
async fn call_id_comes_from_the_request_state() {
    let server = mock(StatusCode::OK, SERVICE_HASHES.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let mut request = data(r#"{"litellm_call_id":"req-1"}"#);
    apply(&guardrail, inputs(FOUR), &mut request).await.unwrap();
    assert!(guardrail.has_issued_hashes("req-1").is_some());
    assert_eq!(
        request.get("litellm_call_id"),
        Some(&GuardrailValue::Text("req-1".to_owned()))
    );
}

#[tokio::test]
async fn an_empty_logging_call_id_falls_to_the_request_state() {
    let server = mock(StatusCode::OK, SERVICE_HASHES.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let target = logging_target("");
    let mut request = data(r#"{"litellm_call_id":"req-1"}"#);
    guardrail
        .apply(
            inputs(FOUR),
            &mut request,
            GuardrailInputType::Request,
            Some(&target),
        )
        .await
        .unwrap();
    assert!(guardrail.has_issued_hashes("req-1").is_some());
    assert_eq!(guardrail.has_issued_hashes(""), None);
}

#[tokio::test]
async fn a_missing_call_id_is_generated_and_written_to_the_request() {
    let server = mock(StatusCode::OK, SERVICE_HASHES.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let mut request = data("{}");
    apply(&guardrail, inputs(FOUR), &mut request).await.unwrap();
    let Some(GuardrailValue::Text(generated)) = request.get("litellm_call_id") else {
        panic!("a generated call id");
    };
    let parsed = Uuid::parse_str(generated).expect("a UUID");
    assert_eq!(
        guardrail.has_issued_hashes(generated),
        Some([SENT_HASH.to_owned()].into_iter().collect())
    );
    assert_ne!(parsed, Uuid::nil());
}

#[tokio::test]
async fn hashes_are_lowercased_from_the_service_response() {
    let server = mock(StatusCode::OK, SERVICE_HASHES.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let target = logging_target("call-1");
    let mut request = data("{}");
    guardrail
        .apply(
            inputs(FOUR),
            &mut request,
            GuardrailInputType::Request,
            Some(&target),
        )
        .await
        .unwrap();
    let issued = guardrail
        .has_issued_hashes("call-1")
        .expect("issued hashes");
    assert_eq!(issued, [SENT_HASH.to_owned()].into_iter().collect());
}

#[tokio::test]
async fn ccr_retrieval_disabled_records_no_hashes_and_adds_no_tool() {
    let server = mock(StatusCode::OK, SERVICE_HASHES.to_owned()).await;
    let input = HeadroomInput {
        ccr_retrieval: Some(false),
        ..default_input(&server.base)
    };
    let guardrail = guardrail_with_clock(input);
    let target = logging_target("call-1");
    let mut request = data("{}");
    let returned = guardrail
        .apply(
            inputs(FOUR),
            &mut request,
            GuardrailInputType::Request,
            Some(&target),
        )
        .await
        .unwrap();
    assert_eq!(guardrail.has_issued_hashes("call-1"), None);
    assert!(returned.tools.is_none());
    assert_eq!(
        structure(&returned),
        json(
            r#"[{"role":"system","content":"sys"},{"role":"user","content":"short"},{"role":"assistant","content":"answer"},{"role":"user","content":"now"}]"#
        )
    );
}

fn guardrail_with_clock(input: HeadroomInput) -> HeadroomGuardrail {
    let (built, _now) = clocked(input);
    built
}

#[tokio::test]
async fn hashes_are_pruned_once_the_ttl_has_passed() {
    let server = mock(StatusCode::OK, SERVICE_HASHES.to_owned()).await;
    let input = HeadroomInput {
        ..default_input(&server.base)
    };
    let (guardrail, now) = clocked(input);
    let target = logging_target("first");
    let mut request = data("{}");
    guardrail
        .apply(
            inputs(FOUR),
            &mut request,
            GuardrailInputType::Request,
            Some(&target),
        )
        .await
        .unwrap();
    assert!(guardrail.has_issued_hashes("first").is_some());
    *now.lock().unwrap() = Duration::from_secs(899);
    let later = logging_target("second");
    guardrail
        .apply(
            inputs(FOUR),
            &mut request,
            GuardrailInputType::Request,
            Some(&later),
        )
        .await
        .unwrap();
    assert!(guardrail.has_issued_hashes("first").is_some());
    *now.lock().unwrap() = Duration::from_secs(900);
    let expired = logging_target("third");
    guardrail
        .apply(
            inputs(FOUR),
            &mut request,
            GuardrailInputType::Request,
            Some(&expired),
        )
        .await
        .unwrap();
    assert_eq!(guardrail.has_issued_hashes("first"), None);
    assert!(guardrail.has_issued_hashes("second").is_some());
}

#[tokio::test]
async fn a_response_without_hashes_does_not_prune() {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let (guardrail, now) = clocked(HeadroomInput {
        ..default_input(&server.base)
    });
    guardrail.issued.record(
        "first",
        [SENT_HASH.to_owned()].into_iter().collect(),
        Duration::ZERO,
    );
    *now.lock().unwrap() = Duration::from_secs(3_600);
    let mut request = data("{}");
    apply(&guardrail, inputs(FOUR), &mut request).await.unwrap();
    assert!(guardrail.has_issued_hashes("first").is_some());
}

#[tokio::test]
async fn the_retrieve_tool_is_added_when_hashes_are_recorded() {
    let server = mock(StatusCode::OK, SERVICE_HASHES.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let mut request = data("{}");
    let returned = apply(&guardrail, inputs(FOUR), &mut request).await.unwrap();
    let tools = serde_json::to_value(returned.tools.as_ref().unwrap()).unwrap();
    assert_eq!(
        tools,
        json(
            r#"[{"type":"function","function":{"name":"headroom_retrieve","description":"Retrieve original content that was compressed by Headroom. Call this when you encounter a compression marker containing a hash.","parameters":{"type":"object","properties":{"hash":{"type":"string","description":"The hex hash from the compression marker."},"query":{"type":"string","description":"Optional search query for BM25-ranked retrieval."}},"required":["hash"]}}}]"#
        )
    );
}

#[tokio::test]
async fn the_retrieve_tool_is_not_duplicated() {
    let server = mock(StatusCode::OK, SERVICE_HASHES.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let given = inputs(
        r#"{"structured_messages":[{"role":"system","content":"sys"},{"role":"user","content":"long text"},{"role":"assistant","content":"answer"},{"role":"user","content":"now"}],"tools":[{"type":"function","function":{"name":"headroom_retrieve","parameters":{}}}]}"#,
    );
    let mut request = data("{}");
    let returned = apply(&guardrail, given, &mut request).await.unwrap();
    let tools = returned.tools.expect("tools");
    assert_eq!(tools.len(), 1);
}

#[tokio::test]
async fn existing_tools_are_kept_and_the_retrieve_tool_is_appended() {
    let server = mock(StatusCode::OK, SERVICE_HASHES.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let given = inputs(
        r#"{"structured_messages":[{"role":"system","content":"sys"},{"role":"user","content":"long text"},{"role":"assistant","content":"answer"},{"role":"user","content":"now"}],"tools":[{"type":"function","function":{"name":"weather"}}]}"#,
    );
    let mut request = data("{}");
    let returned = apply(&guardrail, given, &mut request).await.unwrap();
    let tools = serde_json::to_value(returned.tools.as_ref().unwrap()).unwrap();
    let names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| {
            tool.pointer("/function/name")
                .and_then(serde_json::Value::as_str)
        })
        .collect();
    assert_eq!(names, ["weather", "headroom_retrieve"]);
}

#[tokio::test]
async fn no_tool_is_added_without_hashes() {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let mut request = data("{}");
    let returned = apply(&guardrail, inputs(FOUR), &mut request).await.unwrap();
    assert!(returned.tools.is_none());
}

#[tokio::test]
async fn success_records_statistics_and_the_applied_guardrail() {
    let server = mock(
        StatusCode::OK,
        r#"{"messages":[{"role":"user","content":"short"}],"tokens_before":100,"tokens_after":25,"compression_ratio":4.0,"transforms_applied":["dedupe"],"unused":1}"#
            .to_owned(),
    )
    .await;
    let guardrail = guardrail(&server.base);
    let mut request = data("{}");
    apply(&guardrail, inputs(FOUR), &mut request).await.unwrap();
    let record = first_record(&request);
    assert_eq!(record.as_object().map(serde_json::Map::len), Some(9));
    assert_eq!(
        record.get("guardrail_name"),
        Some(&serde_json::Value::String("headroom".to_owned()))
    );
    assert_eq!(
        record.get("guardrail_provider"),
        Some(&serde_json::Value::String("headroom".to_owned()))
    );
    assert_eq!(
        record.get("guardrail_status"),
        Some(&serde_json::Value::String("success".to_owned()))
    );
    assert_eq!(record.get("guardrail_mode"), Some(&serde_json::Value::Null));
    assert_eq!(
        record.get("masked_entity_count"),
        Some(&serde_json::Value::Null)
    );
    assert!(
        record
            .get("start_time")
            .and_then(serde_json::Value::as_f64)
            .is_some()
    );
    assert!(
        record
            .get("end_time")
            .and_then(serde_json::Value::as_f64)
            .is_some()
    );
    assert!(
        record
            .get("duration")
            .and_then(serde_json::Value::as_f64)
            .is_some()
    );
    assert_eq!(
        record_response(&request),
        json(
            r#"{"tokens_before":100,"tokens_after":25,"tokens_saved":75,"compression_ratio":4.0,"transforms_applied":["dedupe"]}"#
        )
    );
    assert_eq!(applied_names(&request), ["headroom"]);
}

#[tokio::test]
async fn a_failed_compression_records_the_unavailable_error() {
    let server = mock(StatusCode::INTERNAL_SERVER_ERROR, "boom".to_owned()).await;
    let input = fail_open_input(&server.base);
    let guardrail = guardrail_with_clock(input);
    let original = inputs(FOUR);
    let mut request = data("{}");
    let returned = apply(&guardrail, original.clone(), &mut request)
        .await
        .unwrap();
    assert_eq!(returned, original);
    assert_eq!(
        record_status(&request),
        serde_json::Value::String("guardrail_failed_to_respond".to_owned())
    );
    assert_eq!(
        record_response(&request),
        serde_json::json!({"error": UNAVAILABLE_MESSAGE})
    );
    assert_eq!(applied_names(&request), ["headroom"]);
}

#[tokio::test]
async fn records_and_applied_names_go_into_litellm_metadata() {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let mut request = data(r#"{"litellm_metadata":{"keep":true}}"#);
    apply(&guardrail, inputs(FOUR), &mut request).await.unwrap();
    let encoded = serde_json::to_value(&request).unwrap();
    let bucket = encoded.get("litellm_metadata").expect("litellm_metadata");
    assert_eq!(bucket.get("keep"), Some(&serde_json::Value::Bool(true)));
    assert_eq!(
        bucket
            .get("standard_logging_guardrail_information")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1)
    );
    assert!(encoded.get("metadata").is_none());
}

#[tokio::test]
async fn existing_records_and_applied_names_are_kept() {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let mut request = data(
        r#"{"metadata":{"standard_logging_guardrail_information":[{"guardrail_name":"other"}],"applied_guardrails":["other"],"keep":true}}"#,
    );
    apply(&guardrail, inputs(FOUR), &mut request).await.unwrap();
    assert_eq!(records(&request).len(), 2);
    assert_eq!(
        records(&request)
            .first()
            .and_then(|entry| entry.get("guardrail_name")),
        Some(&serde_json::Value::String("other".to_owned()))
    );
    assert_eq!(applied_names(&request), ["other", "headroom"]);
    assert_eq!(
        metadata(&request).get("keep"),
        Some(&serde_json::Value::Bool(true))
    );
}

#[tokio::test]
async fn a_duplicate_applied_name_is_not_repeated() {
    let server = mock(StatusCode::OK, SERVICE_ONE.to_owned()).await;
    let guardrail = guardrail(&server.base);
    let mut request = data(r#"{"metadata":{"applied_guardrails":["headroom"]}}"#);
    apply(&guardrail, inputs(FOUR), &mut request).await.unwrap();
    assert_eq!(applied_names(&request), ["headroom"]);
}
