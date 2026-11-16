use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_context::agentic_callback::error::AgenticHookError;
use betterlite_context::agentic_callback::{
    AgenticCall, AgenticCallback, AgenticGate, AgenticLogging, AgenticLoopCall, AgenticOverrides,
};
use betterlite_context::agentic_loop_plan::{
    AgenticLoopPlan, AgenticLoopRequestPatch, FollowupMessage, JsonMap,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use http::header::AUTHORIZATION;
use http::{HeaderMap, HeaderValue};
use indexmap::IndexMap;
use serde_json::value::RawValue;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use super::super::anthropic_plan::execute_anthropic_agentic_plan;
use super::super::dispatch::call_agentic_completion_hooks;
use super::super::error::{AgenticExecutionError, CallbackError};
use super::super::executor::{
    AnthropicContinuation, CallbackRun, CompletionCall, ContinuationExecutor, PlanRun,
    ResponsesContinuation,
};
use super::super::http::{DEFAULT_DEADLINE, ResponsesHttpExecutor};
use super::super::responses_plan::execute_responses_agentic_plan;
use super::super::values::typed_map;
use super::super::{AgenticResult, LoggingView};
use crate::api::chat_agentic_loop::tests::support::{
    CallbackRecords, Script, ScriptedCallback, StopError, gate, json_map, messages, overrides,
    plan, raw,
};
use crate::api::responses_hook_policy::agentic::AgenticLoopSettings;
use crate::api::responses_hook_policy::safety::{ApiSurface, sort_json_keys};

pub(crate) const PROVIDER_AUTHORIZATION: &str = "Bearer sk-test";
pub(crate) const RESPONSES_BODY: &str =
    r#"{"id":"resp_first","object":"response","created_at":1,"status":"completed","output":[]}"#;
pub(crate) const NEXT_BODY: &str =
    r#"{"id":"resp_next","object":"response","created_at":1,"status":"completed","output":[]}"#;
pub(crate) const ANTHROPIC_BODY: &str = r#"{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn","model":"claude-3-5-sonnet"}"#;
pub(crate) const REFUSED_BODY: &str = r#"{"id":"msg_2","type":"message","role":"assistant","content":[{"type":"tool_use","id":"call-ws","name":"web_search","input":{}},{"type":"text","text":"after"}],"stop_reason":"tool_use"}"#;

pub(crate) fn typed_kwargs(entries: &[(&str, &str)]) -> IndexMap<String, Json> {
    entries
        .iter()
        .map(|(key, value)| {
            let typed = serde_json::from_str(value).expect("a test JSON value");
            ((*key).to_owned(), typed)
        })
        .collect()
}

// the gate hands back either a wrapped list under `tool_calls` or one bare call, as LiteLLM allows
pub(crate) fn refused_tools(name: &str, id: &str) -> JsonMap {
    json_map(&[(
        "tool_calls",
        &format!(r#"[{{"id":"{id}","name":"{name}"}}]"#),
    )])
}

pub(crate) fn continue_messages() -> Vec<FollowupMessage> {
    vec![FollowupMessage(raw(
        r#"{"role":"user","content":"continue"}"#,
    ))]
}

pub(crate) fn fingerprint_of(tools: &JsonMap) -> String {
    let object = Json::Object(typed_map(tools).expect("a tool map"));
    python_json::dumps(&sort_json_keys(&object))
}

#[derive(Clone)]
pub(crate) struct Fixture {
    pub response: Box<RawValue>,
    pub messages: Vec<FollowupMessage>,
    pub optional_params: JsonMap,
    pub kwargs: IndexMap<String, Json>,
    pub agentic_loop_params: Option<JsonMap>,
    pub call_id: Option<&'static str>,
    pub converted: bool,
    pub model: &'static str,
    pub provider: &'static str,
    pub stream: bool,
    pub api_surface: ApiSurface,
}

pub(crate) fn fixture() -> Fixture {
    Fixture {
        response: raw(RESPONSES_BODY),
        messages: messages(),
        optional_params: JsonMap::new(),
        kwargs: IndexMap::new(),
        agentic_loop_params: None,
        call_id: Some("call-1"),
        converted: false,
        model: "gpt-4o",
        provider: "openai",
        stream: false,
        api_surface: ApiSurface::Responses,
    }
}

pub(crate) fn anthropic_fixture() -> Fixture {
    Fixture {
        response: raw(ANTHROPIC_BODY),
        model: "claude-3-5-sonnet",
        api_surface: ApiSurface::AnthropicMessages,
        ..fixture()
    }
}

impl Fixture {
    pub(crate) fn call(&self) -> CompletionCall<'_> {
        let logging = self.call_id.map(|call_id| LoggingView {
            call_id: Some(call_id),
            agentic_loop_params: self.agentic_loop_params.as_ref(),
            websearch_converted_stream: self.converted,
        });
        CompletionCall {
            response: &self.response,
            model: self.model,
            messages: &self.messages,
            optional_params: &self.optional_params,
            logging,
            stream: self.stream,
            custom_llm_provider: self.provider,
            kwargs: &self.kwargs,
            api_surface: self.api_surface,
        }
    }
}

#[derive(Default)]
pub(crate) struct ScriptedExecutor {
    responses: Mutex<Option<Result<Box<RawValue>, StopError>>>,
    anthropic: Mutex<Option<Result<Box<RawValue>, StopError>>>,
    responses_calls: Mutex<Vec<ResponsesContinuation>>,
    anthropic_calls: Mutex<Vec<AnthropicContinuation>>,
}

impl ScriptedExecutor {
    pub(crate) fn responding(body: &str) -> Self {
        let executor = Self::default();
        fill(&executor.responses, Ok(raw(body)));
        executor
    }

    pub(crate) fn anthropic_responding(body: &str) -> Self {
        let executor = Self::default();
        fill(&executor.anthropic, Ok(raw(body)));
        executor
    }

    pub(crate) fn failing(message: &'static str) -> Self {
        let executor = Self::default();
        fill(&executor.responses, Err(StopError { message }));
        executor
    }

    pub(crate) fn responses_calls(&self) -> Vec<ResponsesContinuation> {
        self.responses_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn anthropic_calls(&self) -> Vec<AnthropicContinuation> {
        self.anthropic_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn call_count(&self) -> usize {
        self.responses_calls()
            .len()
            .saturating_add(self.anthropic_calls().len())
    }
}

fn fill(
    slot: &Mutex<Option<Result<Box<RawValue>, StopError>>>,
    answer: Result<Box<RawValue>, StopError>,
) {
    let mut scripted = slot.lock().unwrap_or_else(PoisonError::into_inner);
    *scripted = Some(answer);
}

fn take_script(
    slot: &Mutex<Option<Result<Box<RawValue>, StopError>>>,
) -> Result<Box<RawValue>, StopError> {
    let scripted = slot.lock().unwrap_or_else(PoisonError::into_inner).take();
    scripted.unwrap_or(Err(StopError {
        message: "the executor was called again",
    }))
}

#[async_trait]
impl ContinuationExecutor for ScriptedExecutor {
    type Error = StopError;

    async fn responses(&self, call: &ResponsesContinuation) -> Result<Box<RawValue>, Self::Error> {
        self.responses_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.clone());
        take_script(&self.responses)
    }

    async fn anthropic_messages(
        &self,
        call: &AnthropicContinuation,
    ) -> Result<Box<RawValue>, Self::Error> {
        self.anthropic_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.clone());
        take_script(&self.anthropic)
    }
}

// one scripted callback that also keeps the order of the hooks across every callback in a dispatcher run
pub(crate) struct RecordingCallback {
    name: &'static str,
    inner: Arc<ScriptedCallback>,
    order: Arc<Mutex<Vec<String>>>,
    posts: Mutex<Vec<String>>,
}

pub(crate) fn recording(
    name: &'static str,
    script: Script,
    order: &Arc<Mutex<Vec<String>>>,
) -> Arc<RecordingCallback> {
    Arc::new(RecordingCallback {
        name,
        inner: Arc::new(ScriptedCallback::new(script)),
        order: Arc::clone(order),
        posts: Mutex::new(Vec::new()),
    })
}

impl RecordingCallback {
    pub(crate) fn records(&self) -> CallbackRecords {
        self.inner.records()
    }

    pub(crate) fn posts(&self) -> Vec<String> {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, step: &str) {
        let entry = format!("{}:{step}", self.name);
        self.order
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }
}

#[async_trait]
impl AgenticCallback for RecordingCallback {
    fn callback_name(&self) -> &str {
        self.name
    }

    fn overrides(&self) -> AgenticOverrides {
        self.inner.overrides()
    }

    async fn should_run_agentic_loop(
        &self,
        call: &AgenticCall<'_>,
    ) -> Result<AgenticGate, AgenticHookError> {
        self.record("gate");
        self.inner.should_run_agentic_loop(call).await
    }

    async fn build_agentic_loop_plan(
        &self,
        call: &AgenticLoopCall<'_>,
    ) -> Result<AgenticLoopPlan, AgenticHookError> {
        self.record("plan");
        self.inner.build_agentic_loop_plan(call).await
    }

    async fn run_agentic_loop(
        &self,
        call: &AgenticLoopCall<'_>,
    ) -> Result<Option<Box<RawValue>>, AgenticHookError> {
        self.record("legacy");
        self.inner.run_agentic_loop(call).await
    }

    async fn post_agentic_loop_response(
        &self,
        response: Box<RawValue>,
        plan: &AgenticLoopPlan,
        kwargs: &JsonMap,
    ) -> Result<Box<RawValue>, AgenticHookError> {
        self.record("post");
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(response.get().to_owned());
        self.inner
            .post_agentic_loop_response(response, plan, kwargs)
            .await
    }

    async fn agentic_loop_cleanup(
        &self,
        plan: &AgenticLoopPlan,
        kwargs: &JsonMap,
    ) -> Result<(), AgenticHookError> {
        self.record("cleanup");
        self.inner.agentic_loop_cleanup(plan, kwargs).await
    }
}

pub(crate) fn callback_list(items: &[&Arc<RecordingCallback>]) -> Vec<Arc<dyn AgenticCallback>> {
    let mut collected: Vec<Arc<dyn AgenticCallback>> = Vec::with_capacity(items.len());
    for item in items.iter().copied() {
        let callback: Arc<RecordingCallback> = Arc::clone(item);
        collected.push(callback);
    }
    collected
}

pub(crate) fn order_log(order: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    order.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

pub(crate) async fn dispatch(
    fixture: &Fixture,
    callbacks: &[Arc<dyn AgenticCallback>],
    executor: &ScriptedExecutor,
) -> Result<Option<AgenticResult>, AgenticExecutionError> {
    let call = fixture.call();
    call_agentic_completion_hooks(&call, callbacks, executor).await
}

pub(crate) fn plan_with_patch(request_patch: AgenticLoopRequestPatch) -> AgenticLoopPlan {
    plan(Some(request_patch), None, false, true)
}

pub(crate) fn gated_script(tools: &JsonMap) -> Script {
    Script::default()
        .with_overrides(overrides(true, false, false))
        .with_gate(gate(true, tools.clone()))
}

pub(crate) fn legacy_script(body: &str) -> Script {
    Script::default()
        .with_overrides(overrides(true, false, false))
        .with_gate(gate(true, JsonMap::new()))
        .with_legacy(Some(raw(body)))
}

pub(crate) fn null_legacy_script() -> Script {
    Script::default()
        .with_overrides(overrides(true, false, false))
        .with_gate(gate(true, JsonMap::new()))
        .with_legacy(None)
}

pub(crate) fn plan_script(tools: &JsonMap, request_patch: AgenticLoopRequestPatch) -> Script {
    Script::default()
        .with_overrides(overrides(true, true, false))
        .with_gate(gate(true, tools.clone()))
        .with_plan(plan_with_patch(request_patch))
}

pub(crate) fn continuing_script(tools: &JsonMap) -> Script {
    Script::default()
        .with_overrides(overrides(true, true, false))
        .with_gate(gate(true, tools.clone()))
        .with_plan(plan(None, None, false, false))
}

// the direct plan-executor harness: one callback, one plan, one gate, no dispatcher above it
pub(crate) struct PlanHarness {
    pub fixture: Fixture,
    pub callback: Arc<RecordingCallback>,
    pub state: PlanState,
}

pub(crate) struct PlanState {
    pub gate: AgenticGate,
    pub tool_calls: Json,
    pub settings: AgenticLoopSettings,
    pub fingerprint: String,
    pub hook_kwargs: JsonMap,
    pub provider_kwargs: IndexMap<String, Json>,
    pub logging: Option<AgenticLogging>,
}

impl Default for PlanState {
    fn default() -> Self {
        Self {
            gate: AgenticGate::default(),
            tool_calls: Json::Null,
            settings: AgenticLoopSettings {
                depth: 0,
                max_loops: 3,
                fingerprints: Vec::new(),
            },
            fingerprint: "fingerprint-1".to_owned(),
            hook_kwargs: JsonMap::new(),
            provider_kwargs: IndexMap::new(),
            logging: Some(AgenticLogging {
                call_id: Some("call-1".to_owned()),
                agentic_loop_params: None,
            }),
        }
    }
}

pub(crate) fn plan_harness(fixture: Fixture, script: Script) -> PlanHarness {
    let order = Arc::new(Mutex::new(Vec::new()));
    let callback = recording("cb", script, &order);
    let provider_kwargs = fixture.kwargs.clone();
    let defaults = PlanState::default();
    PlanHarness {
        fixture,
        callback,
        state: PlanState {
            gate: defaults.gate,
            tool_calls: defaults.tool_calls,
            settings: defaults.settings,
            fingerprint: defaults.fingerprint,
            hook_kwargs: defaults.hook_kwargs,
            provider_kwargs,
            logging: defaults.logging,
        },
    }
}

impl PlanHarness {
    pub(crate) async fn responses(
        &self,
        plan: &AgenticLoopPlan,
        executor: &ScriptedExecutor,
    ) -> Result<Box<RawValue>, CallbackError<StopError>> {
        self.execute(plan, executor, ApiSurface::Responses).await
    }

    pub(crate) async fn anthropic(
        &self,
        plan: &AgenticLoopPlan,
        executor: &ScriptedExecutor,
    ) -> Result<Box<RawValue>, CallbackError<StopError>> {
        self.execute(plan, executor, ApiSurface::AnthropicMessages)
            .await
    }

    async fn execute(
        &self,
        plan: &AgenticLoopPlan,
        executor: &ScriptedExecutor,
        surface: ApiSurface,
    ) -> Result<Box<RawValue>, CallbackError<StopError>> {
        let call = self.fixture.call();
        let run = self.callback_run(&call);
        let plan_run = PlanRun { run: &run, plan };
        match surface {
            ApiSurface::Responses => execute_responses_agentic_plan(&plan_run, executor).await,
            ApiSurface::AnthropicMessages => {
                execute_anthropic_agentic_plan(&plan_run, executor).await
            }
        }
    }

    fn callback_run<'run>(&'run self, call: &'run CompletionCall<'run>) -> CallbackRun<'run> {
        CallbackRun {
            call: call.clone(),
            callback: self.callback.as_ref(),
            gate: &self.state.gate,
            tool_calls: &self.state.tool_calls,
            settings: &self.state.settings,
            fingerprint: &self.state.fingerprint,
            hook_kwargs: &self.state.hook_kwargs,
            provider_kwargs: &self.state.provider_kwargs,
            logging: self.state.logging.as_ref(),
        }
    }
}

pub(crate) fn http_executor(api_base: &str) -> ResponsesHttpExecutor {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the proxy policy"),
    );
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_static(PROVIDER_AUTHORIZATION),
    );
    ResponsesHttpExecutor {
        client: ProxiedClient::outbound(&policy).expect("the test client"),
        api_base: format!("{api_base}/v1"),
        provider: ProviderId::Openai,
        headers,
        model_info: None,
        deadline: DEFAULT_DEADLINE,
    }
}

pub(crate) struct SeenRequest {
    pub request_line: String,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

// the mock provider both sides of the oracle share: one scripted answer per request, requests kept
pub(crate) struct Upstream {
    pub(crate) url: String,
    pub(crate) answers: Arc<Mutex<VecDeque<(u16, String)>>>,
    pub(crate) seen: Arc<Mutex<Vec<SeenRequest>>>,
    pub(crate) _tasks: JoinSet<()>,
}

impl Upstream {
    pub(crate) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the test provider binds a loopback port");
        let url = format!(
            "http://{}",
            listener.local_addr().expect("the bound address is known")
        );
        let answers = Arc::new(Mutex::new(VecDeque::new()));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut tasks = JoinSet::new();
        tasks.spawn(serve(listener, Arc::clone(&answers), Arc::clone(&seen)));
        Self {
            url,
            answers,
            seen,
            _tasks: tasks,
        }
    }

    pub(crate) fn answer(&self, status: u16, body: &str) {
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back((status, body.to_owned()));
    }

    pub(crate) fn requests(&self) -> Vec<SeenRequest> {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        let mut requests = Vec::with_capacity(seen.len());
        for request in seen.drain(..) {
            requests.push(request);
        }
        drop(seen);
        requests
    }
}

async fn serve(
    listener: TcpListener,
    answers: Arc<Mutex<VecDeque<(u16, String)>>>,
    seen: Arc<Mutex<Vec<SeenRequest>>>,
) {
    while let Ok((mut socket, _peer)) = listener.accept().await {
        let Some(request) = read_request(&mut socket).await else {
            continue;
        };
        seen.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let answer = answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| (500, String::new()));
        let (status, body) = answer;
        let reply = format!(
            "HTTP/1.1 {status} Answer\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        drop(socket.write_all(reply.as_bytes()).await);
        drop(socket.shutdown().await);
    }
}

async fn read_request(socket: &mut TcpStream) -> Option<SeenRequest> {
    let mut received = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = socket.read(&mut chunk).await.ok()?;
        received.extend_from_slice(chunk.get(..read)?);
        let Some(head_end) = received.windows(4).position(|window| window == b"\r\n\r\n") else {
            if read == 0 {
                return None;
            }
            continue;
        };
        let head = String::from_utf8(received.get(..head_end)?.to_vec()).ok()?;
        let mut lines = head.lines();
        let request_line = lines.next()?.to_owned();
        let mut headers = BTreeMap::new();
        for line in lines {
            let (name, value) = line.split_once(':')?;
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
        let length = headers
            .get("content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let body_start = head_end.checked_add(4)?;
        if received.len().saturating_sub(body_start) >= length || read == 0 {
            let body = String::from_utf8(received.get(body_start..)?.to_vec()).ok()?;
            return Some(SeenRequest {
                request_line,
                headers,
                body,
            });
        }
    }
}
