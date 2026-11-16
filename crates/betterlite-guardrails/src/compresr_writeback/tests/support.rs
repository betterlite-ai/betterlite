use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::compresr_writeback::Configured;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::compresr_settings::{Clock, CompresrGuardrail, CompresrRuntime};
use crate::compresr_writeback::apply_compression;
use crate::generic_guardrail_contracts::{GuardrailInputType, GuardrailMessage};
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};

pub(super) const NOW: Duration = Duration::from_secs(100);

// One user turn, one tool call and its output, the shape the query comes from
pub(super) const TOOL_CONVERSATION: &str = r#"[{"role":"user","content":"find the answer"},{"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"search","arguments":"{\"q\":\"x\"}"}}]},{"role":"tool","tool_call_id":"c1","content":"long tool output one"}]"#;

pub(super) type Requests = Arc<Mutex<Vec<Seen>>>;

#[derive(Debug, Clone)]
pub(super) struct Seen {
    pub(super) path: String,
    pub(super) api_key: String,
    pub(super) body: GuardrailValue,
}

pub(super) struct Server {
    pub(super) base: String,
    pub(super) seen: Requests,
    _tasks: JoinSet<io::Result<()>>,
}

async fn record(
    State((seen, status, reply)): State<(Requests, StatusCode, String)>,
    request: Request,
) -> (StatusCode, String) {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, 1024 * 1024).await.unwrap();
    let parsed: OpaqueParams = serde_json::from_slice(&bytes).unwrap();
    let api_key = parts
        .headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    seen.lock().unwrap().push(Seen {
        path: parts.uri.path().to_owned(),
        api_key,
        body: GuardrailValue::Object(parsed.0),
    });
    (status, reply)
}

pub(super) async fn serve(status: u16, reply: &str) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Requests = Arc::new(Mutex::new(Vec::new()));
    let state = (
        Arc::clone(&seen),
        StatusCode::from_u16(status).unwrap(),
        reply.to_owned(),
    );
    let router = Router::new().fallback(record).with_state(state);
    let mut tasks = JoinSet::new();
    tasks.spawn(axum::serve(listener, router).into_future());
    Server {
        base,
        seen,
        _tasks: tasks,
    }
}

// The service's answer for one target, or a batch of them
pub(super) fn reply(results: &[(&str, i64, i64)]) -> String {
    let rows: Vec<String> = results
        .iter()
        .map(|(text, before, after)| {
            format!(
                r#"{{"compressed_context":"{text}","original_tokens":{before},"compressed_tokens":{after}}}"#
            )
        })
        .collect();
    match rows.as_slice() {
        [single] => format!(r#"{{"data":{single}}}"#),
        _ => format!(r#"{{"data":{{"results":[{}]}}}}"#, rows.join(",")),
    }
}

fn configured(base: &str, params: &str) -> Configured {
    let source = format!(
        "guardrails:\n  - guardrail_name: squeeze\n    litellm_params:\n      guardrail: compresr\n      mode: pre_call\n      api_key: test-key\n      api_base: '{base}'\n      min_chars_to_compress: 5\n{params}"
    );
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&source)
    .unwrap()
    .sections
    .compresr_writeback
    .guardrails
    .into_iter()
    .next()
    .unwrap()
}

pub(super) fn guardrail_at(base: &str, params: &str, clock: Clock) -> CompresrGuardrail {
    let runtime = CompresrRuntime {
        clock,
        event_hook: None,
    };
    CompresrGuardrail::new(
        &configured(base, params),
        &BTreeMap::<String, String>::new(),
        runtime,
        reqwest::Client::default(),
    )
    .unwrap()
}

pub(super) fn guardrail(base: &str, params: &str) -> CompresrGuardrail {
    guardrail_at(base, params, Box::new(|| NOW))
}

pub(super) fn messages(json: &str) -> Vec<GuardrailMessage> {
    serde_json::from_str(json).unwrap()
}

// The inputs of a request with these structured messages and any further fields, written as `,"name":value`
pub(super) fn conversation(rows: &str, extra: &str) -> GuardrailApiInputs {
    let body = format!(r#"{{"structured_messages":{rows}{extra}}}"#);
    serde_json::from_str(&body).unwrap()
}

pub(super) const SCOPE: &str = "scope-a";
pub(super) const CALL_ID: &str = "call-1";
pub(super) const ORIGINAL: &str = "long tool output one";

// A request of the authenticated caller SCOPE, call CALL_ID
pub(super) async fn apply_as_caller(
    guardrail: &CompresrGuardrail,
    input: GuardrailApiInputs,
) -> GuardrailApiInputs {
    let mut target = log_target(SCOPE, CALL_ID);
    apply_compression(
        guardrail,
        input,
        &mut BTreeMap::new(),
        GuardrailInputType::Request,
        Some(&mut target),
    )
    .await
    .unwrap()
}

pub(super) fn field(value: &GuardrailValue, name: &str) -> GuardrailValue {
    let GuardrailValue::Object(fields) = value else {
        panic!("{value:?} is not an object");
    };
    fields.get(name).cloned().unwrap()
}

// What the proxy leaves on the logging object once a virtual key is authenticated
pub(super) fn log_target(scope: &str, call_id: &str) -> GuardrailLogTarget {
    let auth = GuardrailValue::Object(BTreeMap::from([(
        "api_key".to_owned(),
        GuardrailValue::Text(scope.to_owned()),
    )]));
    let metadata = GuardrailValue::Object(BTreeMap::from([("user_api_key_auth".to_owned(), auth)]));
    GuardrailLogTarget {
        litellm_params: Some(BTreeMap::from([("metadata".to_owned(), metadata)])),
        model_call_params: Some(BTreeMap::from([(
            "litellm_call_id".to_owned(),
            GuardrailValue::Text(call_id.to_owned()),
        )])),
    }
}

pub(super) fn content_of(inputs: &GuardrailApiInputs, index: usize) -> GuardrailValue {
    inputs
        .structured_messages
        .as_ref()
        .and_then(|rows| rows.get(index))
        .and_then(|row| row.extra.get("content"))
        .cloned()
        .unwrap()
}

pub(super) fn sent(server: &Server) -> Vec<Seen> {
    server
        .seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}
