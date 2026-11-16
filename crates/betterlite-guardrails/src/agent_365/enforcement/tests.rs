#![cfg(test)]

mod errors;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use indexmap::IndexMap;
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap};
use secrecy::SecretString;
use serde::Deserialize;

use super::pre_mcp_call_hook;
use crate::agent_365_tokens::poster::{Agent365Poster, Agent365PosterError, PostReply};
use crate::agent_365_tokens::{Agent365Guardrail, Agent365Settings, EVALUATE_URL};
use crate::guardrail_base::failure::GuardrailFailure;

type Data = BTreeMap<String, GuardrailValue>;

const ASSERTION: &str = "header.payload.signature";
const OBO_TOKEN: &str = "obo-token";
const TOOL: &str = "get_weather";
const SERVER: &str = "weather";
const CALL_ID: &str = "call-1";
const GUARDRAIL_NAME: &str = "agent-365-guard";
const MISSING_BEARER_REASON: &str = "the caller did not present an Entra bearer token; the Agent 365 guardrail authorizes tool calls On-Behalf-Of the signed-in user";
const TOKEN_ENDPOINT: &str = "https://login.microsoftonline.com/tenant-abc/oauth2/v2.0/token";
const FIXED_NOW_NANOS: u64 = 1_000_000_000_000_000;
const LATENCY_STEP_NANOS: u64 = 123_456_000;
const LATENCY_SECONDS: f64 = 0.123456;
const LATENCY_MILLIS: f64 = 123.5;
const QUARTER_MILLI_NANOS: u64 = 250_000;

struct FakePoster {
    replies: Mutex<VecDeque<ScriptedReply>>,
    seen: Mutex<Vec<RecordedPost>>,
    now: Arc<AtomicU64>,
}

#[derive(Debug, Clone)]
struct RecordedPost {
    url: String,
    authorization: Option<String>,
    content_type: Option<String>,
    body: Vec<u8>,
}

#[derive(Debug, Clone)]
enum ScriptedReply {
    Reply(PostReply),
    Unreachable,
}

impl FakePoster {
    fn new(now: Arc<AtomicU64>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::new()),
            seen: Mutex::new(Vec::new()),
            now,
        }
    }

    fn enqueue(&self, replies: Vec<ScriptedReply>) {
        self.replies.lock().unwrap().extend(replies);
    }

    fn recorded(&self) -> Vec<RecordedPost> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl Agent365Poster for FakePoster {
    async fn post(
        &self,
        url: &str,
        headers: &HeaderMap,
        body: &[u8],
        _timeout: Duration,
    ) -> Result<PostReply, Agent365PosterError> {
        let header = |name| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        self.seen.lock().unwrap().push(RecordedPost {
            url: url.to_owned(),
            authorization: header(AUTHORIZATION),
            content_type: header(CONTENT_TYPE),
            body: body.to_vec(),
        });
        let _advanced = self.now.fetch_add(LATENCY_STEP_NANOS, Ordering::Relaxed);
        match self.replies.lock().unwrap().pop_front() {
            Some(ScriptedReply::Reply(reply)) => Ok(reply),
            Some(ScriptedReply::Unreachable) => Err(Agent365PosterError::NoReply {
                url: url.to_owned(),
            }),
            None => panic!("the scripted replies are exhausted"),
        }
    }
}

struct Harness {
    guardrail: Agent365Guardrail,
    poster: Arc<FakePoster>,
    data: Data,
}

fn clock_handle(now: Arc<AtomicU64>) -> Arc<dyn Fn() -> SystemTime + Send + Sync> {
    let handle = move || {
        let nanos = now.load(Ordering::Relaxed);
        UNIX_EPOCH.checked_add(Duration::from_nanos(nanos)).unwrap()
    };
    Arc::new(handle)
}

fn settings(fallback: UnreachableFallback) -> Agent365Settings {
    Agent365Settings {
        guardrail_name: GUARDRAIL_NAME.to_owned(),
        tenant_id: "tenant-abc".to_owned(),
        client_id: "client-xyz".to_owned(),
        client_secret: SecretString::from("secret-123".to_owned()),
        request_timeout: None,
        unreachable_fallback: fallback,
        event_hook: None,
        default_on: false,
    }
}

fn request_data() -> Data {
    BTreeMap::from([
        (
            "incoming_bearer_token".to_owned(),
            GuardrailValue::Text(ASSERTION.to_owned()),
        ),
        (
            "litellm_call_id".to_owned(),
            GuardrailValue::Text(CALL_ID.to_owned()),
        ),
        (
            "mcp_server_name".to_owned(),
            GuardrailValue::Text(SERVER.to_owned()),
        ),
        (
            "mcp_tool_name".to_owned(),
            GuardrailValue::Text(TOOL.to_owned()),
        ),
    ])
}

fn harness(fallback: UnreachableFallback) -> Harness {
    let now = Arc::new(AtomicU64::new(FIXED_NOW_NANOS));
    let poster = Arc::new(FakePoster::new(Arc::clone(&now)));
    let guardrail = Agent365Guardrail::new(
        settings(fallback),
        Arc::clone(&poster),
        true,
        clock_handle(now),
    )
    .unwrap();
    Harness {
        guardrail,
        poster,
        data: request_data(),
    }
}

fn reply(status: u16, body: &str) -> PostReply {
    PostReply {
        status: StatusCode::from_u16(status).unwrap(),
        body: body.as_bytes().to_vec(),
    }
}

fn token_reply() -> PostReply {
    reply(200, r#"{"access_token":"obo-token","expires_in":3600}"#)
}

fn evaluate(body: &str) -> ScriptedReply {
    ScriptedReply::Reply(reply(200, body))
}

fn token() -> ScriptedReply {
    ScriptedReply::Reply(token_reply())
}

fn exchange_failure(error: &str) -> ScriptedReply {
    ScriptedReply::Reply(reply(
        400,
        &format!(r#"{{"error":"{error}","error_description":"denied"}}"#),
    ))
}

async fn run(guardrail: &Agent365Guardrail, data: &mut Data) -> Result<(), GuardrailFailure> {
    pre_mcp_call_hook(
        guardrail,
        data,
        "call_mcp_tool",
        Some("alias-1"),
        &BTreeSet::new(),
    )
    .await
}

fn record_fields(data: &Data) -> &BTreeMap<String, GuardrailValue> {
    let Some(GuardrailValue::Object(metadata)) = data.get("metadata") else {
        panic!("the guardrail recorded no metadata");
    };
    let Some(GuardrailValue::Array(records)) =
        metadata.get("standard_logging_guardrail_information")
    else {
        panic!("the guardrail recorded no verdict");
    };
    assert_eq!(records.len(), 1);
    let GuardrailValue::Object(fields) = &records[0] else {
        panic!("the verdict record is an object");
    };
    fields
}

fn response(fields: &BTreeMap<String, GuardrailValue>) -> &BTreeMap<String, GuardrailValue> {
    let Some(GuardrailValue::Object(response)) = fields.get("guardrail_response") else {
        panic!("the record has no response");
    };
    response
}

fn text(value: &GuardrailValue) -> &str {
    let GuardrailValue::Text(text) = value else {
        panic!("expected text, got {value:?}");
    };
    text
}

fn decimal(value: &GuardrailValue) -> f64 {
    let GuardrailValue::Decimal(number) = value else {
        panic!("expected a number, got {value:?}");
    };
    *number
}

fn http_detail(failure: &GuardrailFailure) -> (u16, &IndexMap<String, GuardrailValue>) {
    let GuardrailFailure::HttpObject { status, detail } = failure else {
        panic!("expected an HTTP object failure, got {failure:?}");
    };
    (*status, detail)
}

fn detail_text<'detail>(
    detail: &'detail IndexMap<String, GuardrailValue>,
    key: &str,
) -> &'detail str {
    text(detail.get(key).unwrap_or_else(|| {
        panic!("the detail has no {key}");
    }))
}

fn status_of(fields: &BTreeMap<String, GuardrailValue>) -> &str {
    text(fields.get("guardrail_status").unwrap())
}

fn duration_of(fields: &BTreeMap<String, GuardrailValue>) -> &GuardrailValue {
    fields.get("duration").unwrap()
}

#[derive(Debug, Deserialize)]
struct ToolName {
    name: String,
}

#[derive(Debug, Deserialize)]
struct EvaluateBody {
    tool: ToolName,
    #[serde(rename = "serverName")]
    server_name: String,
    #[serde(rename = "conversationId")]
    conversation_id: String,
    arguments: Option<BTreeMap<String, GuardrailValue>>,
    #[serde(rename = "agentId")]
    agent_id: Option<String>,
}

fn evaluate_body(request: &RecordedPost) -> EvaluateBody {
    serde_json::from_slice(&request.body).unwrap()
}

fn form_body(request: &RecordedPost) -> BTreeMap<String, String> {
    url::form_urlencoded::parse(&request.body)
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect()
}

#[tokio::test]
async fn skips_a_call_type_that_is_not_an_mcp_call() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    let outcome = pre_mcp_call_hook(
        &guardrail,
        &mut data,
        "completion",
        Some("alias-1"),
        &BTreeSet::new(),
    )
    .await;
    assert!(outcome.is_ok());
    assert!(poster.recorded().is_empty());
    assert!(!data.contains_key("metadata"));
}

#[tokio::test]
async fn skips_a_suppressed_guardrail() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    let suppressed = BTreeSet::from([GUARDRAIL_NAME.to_owned()]);
    let outcome = pre_mcp_call_hook(
        &guardrail,
        &mut data,
        "mcp_call",
        Some("alias-1"),
        &suppressed,
    )
    .await;
    assert!(outcome.is_ok());
    assert!(poster.recorded().is_empty());
    assert!(!data.contains_key("metadata"));
}

#[tokio::test]
async fn skips_a_request_without_a_tool_name() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    let _removed = data.remove("mcp_tool_name");
    let outcome = run(&guardrail, &mut data).await;
    assert!(outcome.is_ok());
    assert!(poster.recorded().is_empty());
    assert!(!data.contains_key("metadata"));
}

#[tokio::test]
async fn missing_bearer_token_is_a_caller_fault() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailOpen);
    let _removed = data.remove("incoming_bearer_token");
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let (status, detail) = http_detail(&failure);
    assert_eq!(status, 401);
    assert_eq!(
        detail_text(detail, "error"),
        "Agent 365 guardrail rejected the tool call"
    );
    assert_eq!(
        detail_text(detail, "message"),
        format!("Tool call '{TOOL}' was blocked because {MISSING_BEARER_REASON}.")
    );
    assert_eq!(detail_text(detail, "tool"), TOOL);
    assert!(!detail.contains_key("correlation_id"));
    assert!(poster.recorded().is_empty());
    let fields = record_fields(&data);
    assert_eq!(status_of(fields), "guardrail_intervened");
    let recorded = response(fields);
    assert_eq!(text(recorded.get("verdict").unwrap()), "Rejected");
    assert_eq!(text(recorded.get("reason").unwrap()), MISSING_BEARER_REASON);
    assert!(!recorded.contains_key("latency_ms"));
    assert!(!recorded.contains_key("defender_status"));
    assert_eq!(duration_of(fields), &GuardrailValue::Null);
}

#[tokio::test]
async fn opaque_bearer_token_is_a_caller_fault() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    let _previous = data.insert(
        "incoming_bearer_token".to_owned(),
        GuardrailValue::Text("sk-virtual-key".to_owned()),
    );
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let (status, _detail) = http_detail(&failure);
    assert_eq!(status, 401);
    assert!(poster.recorded().is_empty());
}

fn allow_replies() -> Vec<ScriptedReply> {
    vec![
        token(),
        evaluate(
            r#"{"allowed":true,"defender":{"status":"Evaluated","verdict":"Allow"},"correlationId":"corr-1"}"#,
        ),
    ]
}

fn assert_token_request(request: &RecordedPost) {
    assert_eq!(request.url, TOKEN_ENDPOINT);
    assert_eq!(
        request.content_type.as_deref(),
        Some("application/x-www-form-urlencoded")
    );
    let form = form_body(request);
    assert_eq!(form["client_id"], "client-xyz");
    assert_eq!(form["client_secret"], "secret-123");
    assert_eq!(form["assertion"], ASSERTION);
}

#[tokio::test]
async fn allows_an_evaluated_verdict() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    let _previous = data.insert(
        "mcp_arguments".to_owned(),
        GuardrailValue::Object(BTreeMap::from([(
            "city".to_owned(),
            GuardrailValue::Text("Paris".to_owned()),
        )])),
    );
    poster.enqueue(allow_replies());
    let outcome = run(&guardrail, &mut data).await;
    assert!(outcome.is_ok());
    let requests = poster.recorded();
    assert_eq!(requests.len(), 2);
    assert_token_request(&requests[0]);
    assert_eq!(requests[1].url, EVALUATE_URL);
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer obo-token")
    );
    assert_eq!(
        requests[1].content_type.as_deref(),
        Some("application/json")
    );
    let body = evaluate_body(&requests[1]);
    assert_eq!(body.tool.name, TOOL);
    assert_eq!(body.server_name, SERVER);
    assert_eq!(body.conversation_id, CALL_ID);
    assert_eq!(body.agent_id.as_deref(), Some("alias-1"));
    assert_eq!(
        body.arguments.unwrap()["city"],
        GuardrailValue::Text("Paris".to_owned())
    );
}

#[tokio::test]
async fn records_an_evaluated_allow() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    poster.enqueue(allow_replies());
    let outcome = run(&guardrail, &mut data).await;
    assert!(outcome.is_ok());
    let fields = record_fields(&data);
    assert_eq!(text(fields.get("guardrail_name").unwrap()), GUARDRAIL_NAME);
    assert_eq!(text(fields.get("guardrail_provider").unwrap()), "agent_365");
    assert_eq!(text(fields.get("guardrail_mode").unwrap()), "pre_mcp_call");
    assert_eq!(status_of(fields), "success");
    assert_eq!(
        duration_of(fields),
        &GuardrailValue::Decimal(LATENCY_SECONDS)
    );
    let recorded = response(fields);
    assert_eq!(text(recorded.get("verdict").unwrap()), "Allow");
    assert_eq!(text(recorded.get("defender_status").unwrap()), "Evaluated");
    assert_eq!(text(recorded.get("correlation_id").unwrap()), "corr-1");
    assert_eq!(decimal(recorded.get("latency_ms").unwrap()), LATENCY_MILLIS);
    assert!(!recorded.contains_key("reason"));
}

// A latency that lands exactly on a half tenth rounds away from zero here, where Python's round moves it to the even tenth
#[tokio::test]
async fn a_quarter_millisecond_latency_rounds_away_from_zero() {
    let poster = Arc::new(FakePoster::new(Arc::new(AtomicU64::new(0))));
    poster.enqueue(allow_replies());
    let clock = Arc::new(AtomicU64::new(0));
    let step = Arc::clone(&clock);
    let handle: Arc<dyn Fn() -> SystemTime + Send + Sync> = Arc::new(move || {
        let nanos = step.fetch_add(QUARTER_MILLI_NANOS, Ordering::Relaxed);
        UNIX_EPOCH.checked_add(Duration::from_nanos(nanos)).unwrap()
    });
    let guardrail = Agent365Guardrail::new(
        settings(UnreachableFallback::FailClosed),
        Arc::clone(&poster),
        true,
        handle,
    )
    .unwrap();
    let mut data = request_data();
    let outcome = run(&guardrail, &mut data).await;
    assert!(outcome.is_ok());
    let fields = record_fields(&data);
    let recorded = response(fields);
    assert_eq!(decimal(recorded.get("latency_ms").unwrap()), 0.3);
    assert_eq!(duration_of(fields), &GuardrailValue::Decimal(0.00025));
}

#[tokio::test]
async fn agent_id_is_omitted_without_a_key_alias() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    poster.enqueue(vec![
        token(),
        evaluate(r#"{"allowed":true,"defender":{"status":"Evaluated"}}"#),
    ]);
    let outcome =
        pre_mcp_call_hook(&guardrail, &mut data, "mcp_call", None, &BTreeSet::new()).await;
    assert!(outcome.is_ok());
    let request = poster.recorded().remove(1);
    let body = evaluate_body(&request);
    assert!(body.agent_id.is_none());
    assert!(body.arguments.is_none());
}

#[tokio::test]
async fn blocks_a_defender_verdict() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    poster.enqueue(vec![
        token(),
        evaluate(
            r#"{"allowed":false,"defender":{"status":"Evaluated","message":"Prompt injection detected"},"correlationId":"corr-2"}"#,
        ),
    ]);
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let (status, detail) = http_detail(&failure);
    assert_eq!(status, 400);
    assert_eq!(
        detail_text(detail, "error"),
        "Blocked by Microsoft Defender"
    );
    assert_eq!(detail_text(detail, "message"), "Prompt injection detected");
    assert_eq!(detail_text(detail, "tool"), TOOL);
    assert_eq!(detail_text(detail, "correlation_id"), "corr-2");
    let fields = record_fields(&data);
    assert_eq!(status_of(fields), "guardrail_intervened");
    let recorded = response(fields);
    assert_eq!(text(recorded.get("verdict").unwrap()), "Block");
    assert_eq!(text(recorded.get("defender_status").unwrap()), "Evaluated");
    assert_eq!(text(recorded.get("correlation_id").unwrap()), "corr-2");
    assert!(!recorded.contains_key("reason"));
}

#[tokio::test]
async fn blocks_with_the_default_message_without_a_defender_message() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    poster.enqueue(vec![
        token(),
        evaluate(r#"{"allowed":false,"defender":{"status":"Blocked"}}"#),
    ]);
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let (status, detail) = http_detail(&failure);
    assert_eq!(status, 400);
    assert_eq!(
        detail_text(detail, "message"),
        format!(
            "Invocation of '{TOOL}' is blocked by Microsoft Threat Detection policies configured by your administrator."
        )
    );
    assert_eq!(detail.get("correlation_id"), Some(&GuardrailValue::Null));
    let fields = record_fields(&data);
    let recorded = response(fields);
    assert_eq!(text(recorded.get("verdict").unwrap()), "Block");
    assert_eq!(text(recorded.get("defender_status").unwrap()), "Blocked");
    assert!(!recorded.contains_key("correlation_id"));
}

#[tokio::test]
async fn an_unevaluated_allow_falls_back_to_unavailable() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    poster.enqueue(vec![
        token(),
        evaluate(r#"{"allowed":true,"correlationId":"corr-3"}"#),
    ]);
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let (status, detail) = http_detail(&failure);
    assert_eq!(status, 503);
    let reason = "Microsoft Defender did not evaluate the call (defender.status=missing)";
    assert_eq!(
        detail_text(detail, "message"),
        format!(
            "Tool call '{TOOL}' was blocked because {reason} and unreachable_fallback is 'fail_closed'."
        )
    );
    let fields = record_fields(&data);
    assert_eq!(status_of(fields), "guardrail_failed_to_respond");
    let recorded = response(fields);
    assert_eq!(text(recorded.get("verdict").unwrap()), "Unavailable");
    assert_eq!(text(recorded.get("correlation_id").unwrap()), "corr-3");
    assert_eq!(text(recorded.get("reason").unwrap()), reason);
    assert!(!recorded.contains_key("defender_status"));
    assert_eq!(decimal(recorded.get("latency_ms").unwrap()), LATENCY_MILLIS);
}

#[tokio::test]
async fn an_unevaluated_allow_fails_open_when_configured() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailOpen);
    poster.enqueue(vec![
        token(),
        evaluate(r#"{"allowed":true,"defender":{"status":"Skipped"},"correlationId":"corr-4"}"#),
    ]);
    let outcome = run(&guardrail, &mut data).await;
    assert!(outcome.is_ok());
    let fields = record_fields(&data);
    let recorded = response(fields);
    assert_eq!(text(recorded.get("verdict").unwrap()), "Unscanned");
    assert_eq!(text(recorded.get("defender_status").unwrap()), "Skipped");
    assert_eq!(
        text(recorded.get("reason").unwrap()),
        "Microsoft Defender did not evaluate the call (defender.status=Skipped)"
    );
    assert_eq!(status_of(fields), "guardrail_failed_to_respond");
}

#[tokio::test]
async fn malformed_evaluation_json_is_unavailable() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    poster.enqueue(vec![token(), evaluate("not json")]);
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let (_status, detail) = http_detail(&failure);
    assert!(
        detail_text(detail, "message").contains("the Agent 365 endpoint returned a non-JSON body")
    );
    let fields = record_fields(&data);
    let recorded = response(fields);
    assert_eq!(text(recorded.get("verdict").unwrap()), "Unavailable");
    assert_eq!(
        text(recorded.get("reason").unwrap()),
        "the Agent 365 endpoint returned a non-JSON body"
    );
    assert!(!recorded.contains_key("latency_ms"));
    assert_eq!(duration_of(fields), &GuardrailValue::Null);
}

#[tokio::test]
async fn non_object_evaluation_body_is_unavailable() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    poster.enqueue(vec![token(), evaluate("[]")]);
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let (status, detail) = http_detail(&failure);
    assert_eq!(status, 503);
    assert!(
        detail_text(detail, "message")
            .contains("the Agent 365 endpoint returned a non-object JSON body")
    );
}

#[tokio::test]
async fn a_verdict_without_a_boolean_allowed_is_unavailable() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    poster.enqueue(vec![
        token(),
        evaluate(r#"{"allowed":"yes","defender":{"status":"Evaluated"}}"#),
    ]);
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let (status, detail) = http_detail(&failure);
    assert_eq!(status, 503);
    assert!(
        detail_text(detail, "message").contains(
            "the Agent 365 endpoint returned a verdict without a boolean 'allowed' field"
        )
    );
}
