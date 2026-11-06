#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, Uri};
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::check::check_moderation_result;
use super::config::{BaseModerationConfigModel, OpenAiModerationConfigModel};
use super::error::ModerationError;
use super::logging::{
    CallTiming, GuardrailResponse, RequestMetadata, TracingDetail, build_tracing_detail,
    process_outcome,
};
use super::transport::async_make_request;
use super::wire::ModerationResponse;
use super::{
    EventHook, GlobalApiKeys, Inputs, ModerationModel, ModerationParams, OpenAiModerationGuardrail,
    RequestData, SUPPORTED_EVENT_HOOKS,
};
use crate::content_filter::risk_scoring::GuardrailStatus;

const CLOCK: fn() -> Duration = || Duration::from_secs(10);

const FLAGGED_BODY: &str = r#"{"id":"modr-1","model":"omni-moderation-latest","results":[{"flagged":true,"categories":{"violence":true,"hate":false,"harassment":true},"category_scores":{"violence":0.91,"hate":0.01,"harassment":0.62}}]}"#;
const CLEAN_BODY: &str = r#"{"id":"modr-2","model":"omni-moderation-latest","results":[{"flagged":false,"categories":{"violence":false,"hate":false},"category_scores":{"violence":0.01,"hate":0.02}}]}"#;
const EMPTY_BODY: &str = r#"{"id":"m","model":"x","results":[]}"#;
const UNREACHABLE: &str = "http://127.0.0.1:1/v1";

struct Seen {
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
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_by_handler = Arc::clone(&seen);
    let answer = move |uri: Uri, headers: HeaderMap, bytes: Bytes| {
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

fn build(
    params: ModerationParams,
    global_keys: GlobalApiKeys,
    lookup: &BTreeMap<String, String>,
) -> Result<OpenAiModerationGuardrail, ModerationError> {
    OpenAiModerationGuardrail::new(
        params,
        global_keys,
        lookup,
        reqwest::Client::default(),
        CLOCK,
    )
}

fn params(api_key: Option<SecretString>, api_base: Option<String>) -> ModerationParams {
    ModerationParams {
        guardrail_name: "moderation".to_owned(),
        api_key,
        api_base,
        model: None,
        streaming_end_of_stream_only: None,
        streaming_sampling_rate: None,
        timeout: None,
    }
}

fn guardrail(api_base: &str) -> OpenAiModerationGuardrail {
    let key = SecretString::from("sk-test".to_owned());
    let given = params(Some(key), Some(api_base.to_owned()));
    build(given, GlobalApiKeys::default(), &BTreeMap::new()).unwrap()
}

fn response(body: &str) -> ModerationResponse {
    serde_json::from_str(body).unwrap()
}

fn inputs(text: &str) -> Inputs {
    serde_json::from_str(text).unwrap()
}

#[test]
fn init_defaults_cover_base_url_model_streaming_and_timeout() {
    let given = params(Some(SecretString::from("sk".to_owned())), None);
    let built = build(given, GlobalApiKeys::default(), &BTreeMap::new()).unwrap();
    assert_eq!(built.api_base, "https://api.openai.com/v1");
    assert_eq!(built.model, ModerationModel::OmniModerationLatest);
    assert!(!built.streaming_end_of_stream_only);
    assert_eq!(built.streaming_sampling_rate, 5);
    assert_eq!(built.timeout, Duration::from_secs(600));
}

#[test]
fn init_keeps_every_explicit_setting_including_zero_and_false() {
    let params = ModerationParams {
        guardrail_name: "mod".to_owned(),
        api_key: Some(SecretString::from("sk".to_owned())),
        api_base: Some("https://proxy.test/v1".to_owned()),
        model: Some(ModerationModel::TextModerationLatest),
        streaming_end_of_stream_only: Some(true),
        streaming_sampling_rate: Some(0),
        timeout: Some(Duration::from_secs(3)),
    };
    let built = build(params, GlobalApiKeys::default(), &BTreeMap::new()).unwrap();
    assert_eq!(built.guardrail_name, "mod");
    assert_eq!(built.api_base, "https://proxy.test/v1");
    assert_eq!(built.model, ModerationModel::TextModerationLatest);
    assert!(built.streaming_end_of_stream_only);
    assert_eq!(built.streaming_sampling_rate, 0);
    assert_eq!(built.timeout, Duration::from_secs(3));
}

#[test]
fn init_treats_an_empty_api_base_as_unset() {
    let given = params(
        Some(SecretString::from("sk".to_owned())),
        Some(String::new()),
    );
    let built = build(given, GlobalApiKeys::default(), &BTreeMap::new()).unwrap();
    assert_eq!(built.api_base, "https://api.openai.com/v1");
}

#[rstest]
#[case::argument_beats_everything(Some("arg"), Some("env"), Some("global"), Some("openai"), "arg")]
#[case::environment_beats_global_keys(None, Some("env"), Some("global"), Some("openai"), "env")]
#[case::litellm_api_key_beats_openai_key(None, None, Some("global"), Some("openai"), "global")]
#[case::openai_key_is_the_last_resort(None, None, None, Some("openai"), "openai")]
#[case::empty_argument_falls_through(Some(""), Some("env"), None, None, "env")]
#[case::empty_environment_counts_as_unset(None, Some(""), Some("global"), None, "global")]
fn api_key_follows_the_precedence_chain(
    #[case] argument: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] litellm_api_key: Option<&str>,
    #[case] litellm_openai_key: Option<&str>,
    #[case] expected: &str,
) {
    let secret = |value: Option<&str>| value.map(|text| SecretString::from(text.to_owned()));
    let lookup: BTreeMap<String, String> = environment
        .map(|value| ("OPENAI_API_KEY".to_owned(), value.to_owned()))
        .into_iter()
        .collect();
    let given = params(secret(argument), None);
    let global_keys = GlobalApiKeys {
        api_key: secret(litellm_api_key),
        openai_key: secret(litellm_openai_key),
    };
    let built = build(given, global_keys, &lookup).unwrap();
    assert_eq!(built.api_key.expose_secret(), expected);
}

#[test]
fn init_without_any_api_key_fails_with_the_litellm_message() {
    let error = build(
        params(None, None),
        GlobalApiKeys::default(),
        &BTreeMap::new(),
    )
    .unwrap_err();
    assert!(matches!(error, ModerationError::MissingApiKey));
    assert_eq!(
        error.to_string(),
        "OpenAI Moderation: api_key is required. Set OPENAI_API_KEY environment variable or pass it in configuration."
    );
}

#[tokio::test]
async fn make_request_posts_model_and_input_with_bearer_and_json_headers() {
    let server = mock(StatusCode::OK, FLAGGED_BODY.to_owned()).await;
    let parsed = async_make_request(&guardrail(&server.base), "some text")
        .await
        .unwrap();
    assert_eq!(parsed.id, "modr-1");
    let seen = server.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/v1/moderations");
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer sk-test"));
    assert_eq!(seen[0].content_type.as_deref(), Some("application/json"));
    assert_eq!(
        seen[0].body,
        r#"{"model":"omni-moderation-latest","input":"some text"}"#
    );
}

#[tokio::test]
async fn make_request_sends_the_configured_text_model() {
    let server = mock(StatusCode::OK, CLEAN_BODY.to_owned()).await;
    let mut instance = guardrail(&server.base);
    instance.model = ModerationModel::TextModerationLatest;
    async_make_request(&instance, "x").await.unwrap();
    assert_eq!(
        server.seen.lock().unwrap()[0].body,
        r#"{"model":"text-moderation-latest","input":"x"}"#
    );
}

#[rstest]
#[case::unauthorized(StatusCode::UNAUTHORIZED)]
#[case::rate_limited(StatusCode::TOO_MANY_REQUESTS)]
#[case::server_error(StatusCode::INTERNAL_SERVER_ERROR)]
#[case::created_is_not_ok(StatusCode::CREATED)]
#[tokio::test]
async fn make_request_raises_the_upstream_status_on_any_non_200(#[case] status: StatusCode) {
    let server = mock(status, "upstream said no".to_owned()).await;
    let error = async_make_request(&guardrail(&server.base), "x")
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), status);
    assert_eq!(
        serde_json::to_string(&error.detail()).unwrap(),
        r#"{"error":"OpenAI Moderation API request failed","details":"upstream said no"}"#
    );
}

#[tokio::test]
async fn make_request_keeps_the_status_when_the_error_body_is_not_utf8() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let answer = || async { (StatusCode::BAD_GATEWAY, vec![0xff_u8, 0xfe_u8]) };
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, Router::new().fallback(answer)).into_future());
    let error = async_make_request(&guardrail(&base), "x")
        .await
        .unwrap_err();
    assert!(matches!(error, ModerationError::ApiBodyNotUtf8 { .. }));
    assert_eq!(error.status_code(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn make_request_times_out_when_the_server_never_answers() {
    let silent = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", silent.local_addr().unwrap());
    let mut instance = guardrail(&base);
    instance.timeout = Duration::from_millis(100);
    let error = async_make_request(&instance, "x").await.unwrap_err();
    assert!(matches!(error, ModerationError::Timeout { .. }));
    assert!(error.detail().is_none());
}

#[tokio::test]
async fn make_request_reports_an_unreachable_server_as_a_request_error() {
    let error = async_make_request(&guardrail(UNREACHABLE), "x")
        .await
        .unwrap_err();
    assert!(matches!(error, ModerationError::Request { .. }));
}

#[tokio::test]
async fn make_request_rejects_a_body_that_is_not_a_moderation_object() {
    let server = mock(StatusCode::OK, r#"{"id":"x"}"#.to_owned()).await;
    let error = async_make_request(&guardrail(&server.base), "x")
        .await
        .unwrap_err();
    assert!(matches!(error, ModerationError::ResponseJson { .. }));
}

#[tokio::test]
async fn make_request_caps_the_response_size() {
    let server = mock(StatusCode::OK, "a".repeat(2 * 1024 * 1024)).await;
    let error = async_make_request(&guardrail(&server.base), "x")
        .await
        .unwrap_err();
    assert!(matches!(error, ModerationError::ResponseTooLarge { .. }));
}

#[tokio::test]
async fn make_request_keeps_the_status_when_an_error_body_is_oversized() {
    let server = mock(StatusCode::BAD_GATEWAY, "b".repeat(2 * 1024 * 1024)).await;
    let error = async_make_request(&guardrail(&server.base), "x")
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::BAD_GATEWAY);
}

#[test]
fn check_blocks_with_400_listing_violated_categories_in_provider_order() {
    let error = check_moderation_result(&response(FLAGGED_BODY)).unwrap_err();
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(
        serde_json::to_string(&error.detail()).unwrap(),
        concat!(
            r#"{"error":"Violated OpenAI moderation policy","moderation_result":"#,
            r#"{"violated_categories":["violence","harassment"],"#,
            r#""category_scores":{"violence":0.91,"hate":0.01,"harassment":0.62}}}"#
        )
    );
}

#[rstest]
#[case::unflagged_first_result(CLEAN_BODY)]
#[case::no_results(EMPTY_BODY)]
#[case::flagged_missing(r#"{"id":"m","model":"x","results":[{"categories":{"a":true}}]}"#)]
#[case::only_a_later_result_is_flagged(
    r#"{"id":"m","model":"x","results":[{"flagged":false,"categories":{"a":false}},{"flagged":true,"categories":{"a":true}}]}"#
)]
fn check_allows_when_the_first_result_is_not_flagged(#[case] body: &str) {
    check_moderation_result(&response(body)).unwrap();
}

#[rstest]
#[case::missing_scores(
    r#"{"id":"m","model":"x","results":[{"flagged":true,"categories":{"a":true}}]}"#,
    r#"{"violated_categories":["a"],"category_scores":{}}"#
)]
#[case::missing_categories(
    r#"{"id":"m","model":"x","results":[{"flagged":true,"category_scores":{"a":0.5}}]}"#,
    r#"{"violated_categories":[],"category_scores":{"a":0.5}}"#
)]
#[case::null_categories(
    r#"{"id":"m","model":"x","results":[{"flagged":true,"categories":null,"category_scores":null}]}"#,
    r#"{"violated_categories":[],"category_scores":{}}"#
)]
fn check_blocks_a_flagged_first_result_with_partial_detail(
    #[case] body: &str,
    #[case] expected: &str,
) {
    let error = check_moderation_result(&response(body)).unwrap_err();
    let detail = serde_json::to_string(&error.detail()).unwrap();
    assert!(
        detail.ends_with(&format!(r#""moderation_result":{expected}}}"#)),
        "{detail}"
    );
}

#[test]
fn tracing_detail_collects_flagged_categories_from_every_result() {
    let body = r#"{"id":"m","model":"x","results":[{"flagged":false,"categories":{"a":true,"b":false}},{"flagged":true,"categories":{"c":true,"d":true}}]}"#;
    let detail = build_tracing_detail(&GuardrailResponse::Moderation(response(body)));
    assert_eq!(
        detail,
        Some(TracingDetail {
            violation_categories: vec!["a".to_owned(), "c".to_owned(), "d".to_owned()],
        })
    );
}

#[rstest]
#[case::nothing_flagged(GuardrailResponse::Moderation(response(CLEAN_BODY)))]
#[case::no_results(GuardrailResponse::Moderation(response(EMPTY_BODY)))]
#[case::text_response(GuardrailResponse::Text("allow".to_owned()))]
fn tracing_detail_is_none_without_flagged_categories(
    #[case] guardrail_response: GuardrailResponse,
) {
    assert_eq!(build_tracing_detail(&guardrail_response), None);
}

#[tokio::test]
async fn apply_guardrail_moderates_the_last_user_block_not_the_texts() {
    let server = mock(StatusCode::OK, CLEAN_BODY.to_owned()).await;
    let given = inputs(
        r#"{"texts":["ignored text"],"structured_messages":[
            {"role":"user","content":"old question"},
            {"role":"assistant","content":"answer"},
            {"role":"user","content":"first part"},
            {"role":"user","content":"second part"}]}"#,
    );
    guardrail(&server.base)
        .apply_guardrail(given, &mut RequestData::default())
        .await
        .unwrap();
    assert_eq!(
        server.seen.lock().unwrap()[0].body,
        r#"{"model":"omni-moderation-latest","input":"first part\nsecond part"}"#
    );
}

#[tokio::test]
async fn apply_guardrail_joins_texts_with_newlines_without_structured_messages() {
    let server = mock(StatusCode::OK, CLEAN_BODY.to_owned()).await;
    guardrail(&server.base)
        .apply_guardrail(
            inputs(r#"{"texts":["one","two","three"]}"#),
            &mut RequestData::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        server.seen.lock().unwrap()[0].body,
        r#"{"model":"omni-moderation-latest","input":"one\ntwo\nthree"}"#
    );
}

#[tokio::test]
async fn apply_guardrail_falls_back_to_texts_when_no_user_message_exists() {
    let server = mock(StatusCode::OK, CLEAN_BODY.to_owned()).await;
    let given = inputs(
        r#"{"texts":["from texts"],"structured_messages":[{"role":"assistant","content":"only assistant"}]}"#,
    );
    guardrail(&server.base)
        .apply_guardrail(given, &mut RequestData::default())
        .await
        .unwrap();
    assert_eq!(
        server.seen.lock().unwrap()[0].body,
        r#"{"model":"omni-moderation-latest","input":"from texts"}"#
    );
}

#[rstest]
#[case::nothing_at_all("{}")]
#[case::empty_texts(r#"{"texts":[]}"#)]
#[case::empty_string_text(r#"{"texts":[""]}"#)]
#[tokio::test]
async fn apply_guardrail_skips_the_api_when_there_is_no_text(#[case] text: &str) {
    let given = inputs(text);
    let mut request_data = RequestData::default();
    let returned = guardrail(UNREACHABLE)
        .apply_guardrail(given.clone(), &mut request_data)
        .await
        .unwrap();
    assert_eq!(returned, given);
    let record = &request_data.metadata.guardrail_records[0];
    assert_eq!(
        record.guardrail_response,
        GuardrailResponse::Text("allow".to_owned())
    );
    assert_eq!(record.guardrail_status, GuardrailStatus::Success);
}

#[tokio::test]
async fn apply_guardrail_returns_the_inputs_unchanged_with_unknown_keys() {
    let server = mock(StatusCode::OK, CLEAN_BODY.to_owned()).await;
    let given =
        inputs(r#"{"texts":["hello"],"images":["data:image/png;base64,AAAA"],"model":"gpt-4o"}"#);
    let returned = guardrail(&server.base)
        .apply_guardrail(given.clone(), &mut RequestData::default())
        .await
        .unwrap();
    assert_eq!(returned, given);
    assert_eq!(
        returned.rest.get("model"),
        Some(&GuardrailValue::Text("gpt-4o".to_owned()))
    );
}

#[tokio::test]
async fn moderate_stashes_the_full_response_under_the_metadata_key() {
    let server = mock(StatusCode::OK, CLEAN_BODY.to_owned()).await;
    let mut metadata = RequestMetadata::default();
    guardrail(&server.base)
        .moderate(&inputs(r#"{"texts":["hi"]}"#), &mut metadata)
        .await
        .unwrap();
    assert_eq!(
        metadata.openai_moderation_response,
        Some(response(CLEAN_BODY))
    );
    let stored = serde_json::to_string(&metadata).unwrap();
    assert!(stored.starts_with(r#"{"_openai_moderation_response":{"id":"modr-2""#));
}

#[tokio::test]
async fn moderate_stashes_the_response_even_when_the_content_is_blocked() {
    let server = mock(StatusCode::OK, FLAGGED_BODY.to_owned()).await;
    let mut metadata = RequestMetadata::default();
    let error = guardrail(&server.base)
        .moderate(&inputs(r#"{"texts":["bad"]}"#), &mut metadata)
        .await
        .unwrap_err();
    assert!(matches!(error, ModerationError::Flagged { .. }));
    assert_eq!(
        metadata.openai_moderation_response,
        Some(response(FLAGGED_BODY))
    );
}

#[tokio::test]
async fn apply_guardrail_logs_success_with_the_popped_response_and_timing() {
    let server = mock(StatusCode::OK, CLEAN_BODY.to_owned()).await;
    let mut request_data = RequestData::default();
    guardrail(&server.base)
        .apply_guardrail(inputs(r#"{"texts":["hi"]}"#), &mut request_data)
        .await
        .unwrap();
    let metadata = &request_data.metadata;
    assert_eq!(metadata.openai_moderation_response, None);
    assert_eq!(metadata.guardrail_records.len(), 1);
    let record = &metadata.guardrail_records[0];
    assert_eq!(record.guardrail_name, "moderation");
    assert_eq!(record.guardrail_status, GuardrailStatus::Success);
    assert_eq!(
        record.guardrail_response,
        GuardrailResponse::Moderation(response(CLEAN_BODY))
    );
    assert_eq!(record.tracing_detail, None);
    let logged = serde_json::to_string(record).unwrap();
    assert!(logged.ends_with(r#""start_time":10.0,"end_time":10.0,"duration":0.0}"#));
}

#[tokio::test]
async fn apply_guardrail_blocks_with_400_and_logs_an_intervention() {
    let server = mock(StatusCode::OK, FLAGGED_BODY.to_owned()).await;
    let mut request_data = RequestData::default();
    let error = guardrail(&server.base)
        .apply_guardrail(inputs(r#"{"texts":["bad"]}"#), &mut request_data)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(request_data.metadata.openai_moderation_response, None);
    let record = &request_data.metadata.guardrail_records[0];
    assert_eq!(
        record.guardrail_status,
        GuardrailStatus::GuardrailIntervened
    );
    assert_eq!(
        record.guardrail_response,
        GuardrailResponse::Moderation(response(FLAGGED_BODY))
    );
    let logged = serde_json::to_string(record).unwrap();
    assert!(logged.contains(r#""violation_categories":["violence","harassment"]"#));
}

#[rstest]
#[case::forbidden(StatusCode::FORBIDDEN, GuardrailStatus::GuardrailFailedToRespond)]
#[case::unprocessable(
    StatusCode::UNPROCESSABLE_ENTITY,
    GuardrailStatus::GuardrailFailedToRespond
)]
#[case::bad_request(StatusCode::BAD_REQUEST, GuardrailStatus::GuardrailFailedToRespond)]
#[case::unauthorized(StatusCode::UNAUTHORIZED, GuardrailStatus::GuardrailFailedToRespond)]
#[case::server_error(
    StatusCode::INTERNAL_SERVER_ERROR,
    GuardrailStatus::GuardrailFailedToRespond
)]
#[tokio::test]
async fn upstream_failures_are_classified_by_status_and_reraised(
    #[case] status: StatusCode,
    #[case] expected: GuardrailStatus,
) {
    let server = mock(status, "denied".to_owned()).await;
    let mut request_data = RequestData::default();
    let error = guardrail(&server.base)
        .apply_guardrail(inputs(r#"{"texts":["hi"]}"#), &mut request_data)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), status);
    let record = &request_data.metadata.guardrail_records[0];
    assert_eq!(record.guardrail_status, expected);
    assert_eq!(
        record.guardrail_response,
        GuardrailResponse::Text(error.to_string())
    );
}

#[test]
fn process_error_prefers_a_stashed_response_over_the_error_text() {
    let mut metadata = RequestMetadata {
        openai_moderation_response: Some(response(FLAGGED_BODY)),
        guardrail_records: Vec::new(),
        rest: BTreeMap::new(),
    };
    let timing = CallTiming {
        started: Duration::from_secs(1),
        ended: Duration::from_secs(4),
    };
    process_outcome(
        "moderation",
        Err(&ModerationError::MissingApiKey),
        &mut metadata,
        timing,
    );
    let record = &metadata.guardrail_records[0];
    assert_eq!(
        record.guardrail_response,
        GuardrailResponse::Moderation(response(FLAGGED_BODY))
    );
    assert_eq!(
        record.guardrail_status,
        GuardrailStatus::GuardrailFailedToRespond
    );
    let logged = serde_json::to_string(record).unwrap();
    assert!(logged.contains(r#""start_time":1.0,"end_time":4.0,"duration":3.0"#));
    assert_eq!(metadata.openai_moderation_response, None);
}

#[test]
fn process_response_falls_back_to_allow_when_nothing_was_stashed() {
    let mut metadata = RequestMetadata::default();
    let timing = CallTiming {
        started: Duration::from_secs(2),
        ended: Duration::from_secs(5),
    };
    process_outcome("moderation", Ok(()), &mut metadata, timing);
    let record = &metadata.guardrail_records[0];
    assert_eq!(
        record.guardrail_response,
        GuardrailResponse::Text("allow".to_owned())
    );
    assert_eq!(record.guardrail_status, GuardrailStatus::Success);
    assert_eq!(record.tracing_detail, None);
    let logged = serde_json::to_string(record).unwrap();
    assert!(logged.ends_with(r#""start_time":2.0,"end_time":5.0,"duration":3.0}"#));
}

#[test]
fn request_data_with_null_metadata_keeps_other_keys_and_stays_usable() {
    let mut request_data: RequestData =
        serde_json::from_str(r#"{"metadata":null,"model":"gpt-4o"}"#).unwrap();
    assert_eq!(request_data.metadata, RequestMetadata::default());
    assert_eq!(
        request_data.rest.get("model"),
        Some(&GuardrailValue::Text("gpt-4o".to_owned()))
    );
    request_data.metadata.openai_moderation_response = Some(response(CLEAN_BODY));
    let stored = serde_json::to_string(&request_data).unwrap();
    assert!(stored.contains(r#""model":"gpt-4o""#));
    assert!(stored.contains(r#""_openai_moderation_response":{"id":"modr-2""#));
}

#[test]
fn request_metadata_keeps_unrelated_keys_when_the_stash_is_popped() {
    let text = format!(r#"{{"user_api_key":"abc","_openai_moderation_response":{CLEAN_BODY}}}"#);
    let mut metadata: RequestMetadata = serde_json::from_str(&text).unwrap();
    assert!(metadata.openai_moderation_response.take().is_some());
    assert_eq!(
        serde_json::to_string(&metadata).unwrap(),
        r#"{"user_api_key":"abc"}"#
    );
}

#[test]
fn supported_event_hooks_are_pre_during_and_post_call() {
    assert_eq!(
        SUPPORTED_EVENT_HOOKS,
        [
            EventHook::PreCall,
            EventHook::DuringCall,
            EventHook::PostCall
        ]
    );
    assert_eq!(
        serde_json::to_string(&SUPPORTED_EVENT_HOOKS).unwrap(),
        r#"["pre_call","during_call","post_call"]"#
    );
}

#[test]
fn config_model_defaults_match_litellm() {
    let config = OpenAiModerationConfigModel::default();
    assert!(config.api_key.is_none());
    assert_eq!(config.api_base, "https://api.openai.com/v1");
    assert_eq!(config.model, ModerationModel::OmniModerationLatest);
    assert!(!config.streaming_end_of_stream_only);
    assert_eq!(config.streaming_sampling_rate, 5);
    assert_eq!(
        OpenAiModerationConfigModel::ui_friendly_name(),
        "OpenAI Moderation"
    );
}

#[test]
fn config_model_reads_explicit_values_and_treats_null_as_default() {
    let explicit: OpenAiModerationConfigModel = serde_json::from_str(
        r#"{"api_key":"sk","api_base":"https://proxy.test/v1","model":"text-moderation-latest","streaming_end_of_stream_only":true,"streaming_sampling_rate":0}"#,
    )
    .unwrap();
    assert_eq!(
        explicit
            .api_key
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("sk")
    );
    assert_eq!(explicit.api_base, "https://proxy.test/v1");
    assert_eq!(explicit.model, ModerationModel::TextModerationLatest);
    assert!(explicit.streaming_end_of_stream_only);
    assert_eq!(explicit.streaming_sampling_rate, 0);
    let nulls: OpenAiModerationConfigModel = serde_json::from_str(
        r#"{"api_key":null,"api_base":null,"model":null,"streaming_end_of_stream_only":null,"streaming_sampling_rate":null}"#,
    )
    .unwrap();
    let defaults = OpenAiModerationConfigModel::default();
    assert!(nulls.api_key.is_none());
    assert_eq!(nulls.api_base, defaults.api_base);
    assert_eq!(nulls.model, defaults.model);
    assert_eq!(
        nulls.streaming_end_of_stream_only,
        defaults.streaming_end_of_stream_only
    );
    assert_eq!(nulls.streaming_sampling_rate, 5);
}

#[test]
fn config_model_rejects_an_unknown_moderation_model_naming_the_value() {
    let error =
        serde_json::from_str::<OpenAiModerationConfigModel>(r#"{"model":"gpt-4"}"#).unwrap_err();
    assert!(error.to_string().contains("gpt-4"));
}

#[rstest]
#[case::absent("{}", ModerationModel::OmniModerationLatest)]
#[case::null(r#"{"model":null}"#, ModerationModel::OmniModerationLatest)]
#[case::text_model(
    r#"{"model":"text-moderation-latest"}"#,
    ModerationModel::TextModerationLatest
)]
fn base_config_model_defaults_the_model_to_omni(
    #[case] text: &str,
    #[case] expected: ModerationModel,
) {
    let base: BaseModerationConfigModel = serde_json::from_str(text).unwrap();
    assert_eq!(base.model, expected);
}
