use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_context::agentic_callback::error::AgenticHookError;
use betterlite_context::agentic_callback::{
    AgenticCall, AgenticCallback as _, AgenticLogging, AgenticLoopCall,
};
use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, FollowupMessage, JsonMap};
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use serde_json::value::RawValue;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;
use uuid::Uuid;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract as _;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::headroom::headroom_compression::{Clock, HeadroomGuardrail, HeadroomRuntime};
use crate::headroom_settings::{HeadroomInput, HeadroomSettings};
use crate::primitives::json_value::JsonValue;

pub(crate) const HASH: &str = "abcdef123456";
pub(crate) const OTHER_HASH: &str = "0123456789ab";
pub(crate) const CALL_ID: &str = "call-1";
pub(crate) const API_KEY: &str = "sk-test";
pub(crate) const RETRIEVE_TOOL: &str = r#"{"type":"function","function":{"name":"headroom_retrieve","description":"Retrieve original content.","parameters":{"type":"object","properties":{"hash":{"type":"string"},"query":{"type":"string"}},"required":["hash"]}}}"#;

// one compressible row: LiteLLM protects the system, the last user and every assistant row
const SEED_INPUTS: &str = r#"{"structured_messages":[{"role":"system","content":"sys"},{"role":"user","content":"long text"},{"role":"assistant","content":"answer"},{"role":"user","content":"now"}]}"#;

#[derive(Clone, Debug)]
pub(crate) struct Seen {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) authorization: Option<String>,
    pub(crate) content_type: Option<String>,
}

#[derive(Clone)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) body: String,
    pub(crate) delay: Option<Duration>,
}

pub(crate) fn reply(status: u16, body: &str) -> Reply {
    Reply {
        status,
        body: body.to_owned(),
        delay: None,
    }
}

pub(crate) fn delayed(status: u16, body: &str, delay: Duration) -> Reply {
    Reply {
        status,
        body: body.to_owned(),
        delay: Some(delay),
    }
}

pub(crate) fn compress_reply(hashes: &[&str]) -> Reply {
    let listed: Vec<String> = hashes.iter().map(|hash| format!("\"{hash}\"")).collect();
    reply(
        200,
        &format!(
            "{{\"messages\":[{{\"role\":\"user\",\"content\":\"short\"}}],\"ccr_hashes\":[{}]}}",
            listed.join(",")
        ),
    )
}

pub(crate) fn retrieve_reply(content: &str) -> Reply {
    let encoded = serde_json::to_string(content).unwrap();
    reply(200, &format!("{{\"original_content\":{encoded}}}"))
}

pub(crate) struct Server {
    pub(crate) base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    _tasks: JoinSet<io::Result<()>>,
}

impl Server {
    pub(crate) fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

pub(crate) async fn start(replies: Vec<Reply>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let answers = Arc::new(Mutex::new(VecDeque::from(replies)));
    let recorder = Arc::clone(&seen);
    let answer = move |method: Method, uri: Uri, headers: HeaderMap| {
        let capture = Arc::clone(&recorder);
        let queue = Arc::clone(&answers);
        async move {
            record(&capture, &method, &uri, &headers);
            let next = queue
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| reply(500, "{}"));
            if let Some(delay) = next.delay {
                sleep(delay).await;
            }
            (StatusCode::from_u16(next.status).unwrap(), next.body)
        }
    };
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, Router::new().fallback(answer)).into_future());
    Server {
        base,
        seen,
        _tasks: server,
    }
}

fn record(recorder: &Mutex<Vec<Seen>>, method: &Method, uri: &Uri, headers: &HeaderMap) {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    recorder.lock().unwrap().push(Seen {
        method: method.to_string(),
        path: uri.to_string(),
        authorization: header("authorization"),
        content_type: header("content-type"),
    });
}

pub(crate) struct Harness {
    pub(crate) guardrail: HeadroomGuardrail,
    pub(crate) now: Arc<Mutex<Duration>>,
}

impl Harness {
    pub(crate) async fn seed(&self, call_id: &str, hashes: &[&str]) {
        let seed: GuardrailApiInputs = serde_json::from_str(SEED_INPUTS).unwrap();
        let mut request = BTreeMap::from([(
            "litellm_call_id".to_owned(),
            GuardrailValue::Text(call_id.to_owned()),
        )]);
        self.guardrail
            .apply_guardrail(seed, &mut request, GuardrailInputType::Request, None)
            .await
            .unwrap();
        let expected: BTreeSet<String> = hashes.iter().map(|hash| (*hash).to_owned()).collect();
        assert_eq!(self.guardrail.has_issued_hashes(call_id), Some(expected));
    }

    pub(crate) fn set_now(&self, now: Duration) {
        *self.now.lock().unwrap() = now;
    }
}

const fn epoch() -> SystemTime {
    UNIX_EPOCH
}

pub(crate) fn harness(base: &str) -> Harness {
    harness_with_timeout(base, None)
}

pub(crate) fn harness_with_timeout(base: &str, timeout: Option<f64>) -> Harness {
    let now = Arc::new(Mutex::new(Duration::ZERO));
    build(base, timeout, &now)
}

fn build(base: &str, timeout: Option<f64>, now: &Arc<Mutex<Duration>>) -> Harness {
    let input = HeadroomInput {
        api_base: Some(base.to_owned()),
        api_key: Some(SecretString::from(API_KEY)),
        model: None,
        unreachable_fallback: None,
        timeout,
        ccr_retrieval: None,
    };
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let settings = HeadroomSettings::new(&input, &environment).unwrap();
    let shared = Arc::clone(now);
    let clock: Clock = Box::new(move || *shared.lock().unwrap());
    let guardrail = HeadroomGuardrail::new(
        Some("headroom".to_owned()),
        settings,
        Box::new(|connect: Duration| base_builder().connect_timeout(connect).build()),
        HeadroomRuntime {
            clock,
            wall: epoch,
            new_id: Uuid::new_v4,
            event_hook: None,
        },
    )
    .unwrap();
    Harness {
        guardrail,
        now: Arc::clone(now),
    }
}

pub(crate) fn raw(text: &str) -> Box<RawValue> {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn message(text: &str) -> FollowupMessage {
    FollowupMessage(raw(text))
}

pub(crate) fn json_map(pairs: &[(&str, &str)]) -> JsonMap {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), raw(value)))
        .collect()
}

pub(crate) fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn request_tool() -> FollowupMessage {
    message(RETRIEVE_TOOL)
}

pub(crate) fn tool_call(id: &str, arguments: &str) -> String {
    format!(
        r#"{{"id":"{id}","type":"function","name":"headroom_retrieve","arguments":{arguments}}}"#
    )
}

pub(crate) fn gate(entries: &[String]) -> JsonMap {
    let listed = format!("[{}]", entries.join(","));
    json_map(&[("tool_calls", listed.as_str())])
}

pub(crate) fn plan_messages(plan: &AgenticLoopPlan) -> Vec<JsonValue> {
    let patch = plan.request_patch.as_ref().expect("a request patch");
    patch
        .messages
        .as_ref()
        .expect("follow-up messages")
        .iter()
        .map(|message| json(message.0.get()))
        .collect()
}

pub(crate) fn values(map: &JsonMap) -> JsonValue {
    json(&serde_json::to_string(map).unwrap())
}

pub(crate) struct GateCall {
    pub(crate) response: Box<RawValue>,
    pub(crate) model: String,
    pub(crate) messages: Vec<FollowupMessage>,
    pub(crate) tools: Option<Vec<FollowupMessage>>,
    pub(crate) stream: bool,
    pub(crate) custom_llm_provider: String,
    pub(crate) kwargs: JsonMap,
}

impl GateCall {
    pub(crate) fn view(&self) -> AgenticCall<'_> {
        let response = &self.response;
        let model = &self.model;
        let messages = &self.messages;
        let tools = self.tools.as_deref();
        AgenticCall {
            response,
            model,
            messages,
            tools,
            stream: self.stream,
            custom_llm_provider: &self.custom_llm_provider,
            kwargs: &self.kwargs,
        }
    }
}

pub(crate) fn gate_call(response: &str) -> GateCall {
    let parsed = raw(response);
    let tools = Some(vec![request_tool()]);
    let model = "gpt-4o".to_owned();
    let custom_llm_provider = "openai".to_owned();
    GateCall {
        response: parsed,
        model,
        messages: Vec::new(),
        tools,
        stream: false,
        custom_llm_provider,
        kwargs: JsonMap::new(),
    }
}

pub(crate) struct LoopCall {
    pub(crate) tools: JsonMap,
    pub(crate) model: String,
    pub(crate) messages: Vec<FollowupMessage>,
    pub(crate) response: Box<RawValue>,
    pub(crate) optional_params: JsonMap,
    pub(crate) logging: Option<AgenticLogging>,
    pub(crate) stream: bool,
    pub(crate) kwargs: JsonMap,
}

impl LoopCall {
    pub(crate) fn view(&self) -> AgenticLoopCall<'_> {
        AgenticLoopCall {
            tools: &self.tools,
            model: &self.model,
            messages: &self.messages,
            response: &self.response,
            optional_params: &self.optional_params,
            logging: self.logging.as_ref(),
            stream: self.stream,
            kwargs: &self.kwargs,
        }
    }
}

pub(crate) fn loop_call(response: &str) -> LoopCall {
    LoopCall {
        tools: gate(&[tool_call("toolu_1", r#"{"hash":"abcdef123456"}"#)]),
        model: "gpt-4o".to_owned(),
        messages: vec![message(r#"{"role":"user","content":"hello"}"#)],
        response: raw(response),
        optional_params: JsonMap::new(),
        logging: None,
        stream: false,
        kwargs: json_map(&[("litellm_call_id", "\"call-1\"")]),
    }
}

pub(crate) async fn build_plan(harness: &Harness, call: &LoopCall) -> AgenticLoopPlan {
    harness
        .guardrail
        .build_agentic_loop_plan(&call.view())
        .await
        .unwrap()
}

pub(crate) async fn plan_error(harness: &Harness, call: &LoopCall) -> AgenticHookError {
    harness
        .guardrail
        .build_agentic_loop_plan(&call.view())
        .await
        .unwrap_err()
}
