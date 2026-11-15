use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::error::Error as StdError;
use std::future::{Future, IntoFuture as _};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderMap, StatusCode, Uri};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_prompts::message_content::Message;
use indexmap::IndexMap;
use reqwest::Client;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tracing::subscriber::set_default;

use super::super::error_handling::HookError;
use super::super::{Data, HookContext, ModelArmorHooks};
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::invocation::GuardrailInvocation;
use crate::guardrail_records::record::{GuardrailInvocationRecord, GuardrailSpanSink};
use crate::model_armor::config::{ModelArmorGuardrail, ModelArmorParams};
use crate::model_armor::decisions::model_response::{CallResponse, ModelResponse, ResponseChoice};
use crate::model_armor::request_files::FileScanSettings;
use crate::model_armor::transport::ModelArmorTransport;
use crate::model_armor::transport::error::ArmorTokenError;
use crate::model_armor::transport::project::ResolvedProject;
use crate::model_armor::transport::tokens::ArmorTokens;

pub(crate) const NAME: &str = "armor";
pub(crate) const PROMPT: &str = "hello";
pub(crate) const CLEAN: &str = r#"{"sanitizationResult":{"filterResults":{}}}"#;
pub(crate) const BLOCKED: &str = r#"{"sanitizationResult":{"filterResults":{"rai":{"raiFilterResult":{"matchState":"MATCH_FOUND"}}}}}"#;
pub(crate) const SANITIZED: &str = r#"{"sanitizationResult":{"filterResults":{"sdp":{"sdpFilterResult":{"deidentifyResult":{"matchState":"MATCH_FOUND","data":{"text":"masked text"}}}}}}}"#;
pub(crate) const SANITIZED_TEXT: &str = r#"{"sanitizedText":"masked text"}"#;
pub(crate) const SANITIZED_SAME: &str = r#"{"sanitizedText":"hello"}"#;
pub(crate) const API_ERROR: &str = r#"{"error":"bad"}"#;
pub(crate) const RECORDS: &str = "standard_logging_guardrail_information";
pub(crate) const STANDARD: &str = "standard_logging_object";

pub(crate) struct StubTokens;

#[async_trait]
impl ArmorTokens for StubTokens {
    type Error = ArmorTokenError;

    async fn access_token(&self) -> Result<(SecretString, Option<String>), ArmorTokenError> {
        Ok((SecretString::from("token"), None))
    }
}

pub(crate) struct DiscardSink;

impl GuardrailSpanSink for DiscardSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct Recorded {
    pub(crate) path: String,
    pub(crate) body: GuardrailValue,
}

type Server = JoinSet<Result<(), io::Error>>;

fn client() -> Client {
    Client::default()
}

async fn serve(replies: Vec<(StatusCode, String)>) -> (String, Arc<Mutex<Vec<Recorded>>>, Server) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let endpoint = format!("http://{}", listener.local_addr().expect("address"));
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let queue = Arc::new(Mutex::new(VecDeque::from(replies)));
    let handler_recorded = Arc::clone(&recorded);
    let handler_queue = Arc::clone(&queue);
    let router = Router::new()
        .fallback(move |_headers: HeaderMap, uri: Uri, body: Bytes| {
            let requests = Arc::clone(&handler_recorded);
            let reply_queue = Arc::clone(&handler_queue);
            async move {
                let parsed = serde_json::from_slice(&body).unwrap_or(GuardrailValue::Null);
                requests.lock().expect("recorded").push(Recorded {
                    path: uri.path().to_owned(),
                    body: parsed,
                });
                let reply = reply_queue.lock().expect("replies").pop_front();
                let (status, payload) = reply.unwrap_or_else(|| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "no reply queued".to_owned(),
                    )
                });
                (status, Body::from(payload))
            }
        })
        .layer(DefaultBodyLimit::disable());
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (endpoint, recorded, server)
}

const TEMPLATE_ID: &str = "template";
const PROJECT_ID: &str = "project";
const LOCATION: &str = "us-central1";

fn transport(guardrail: &ModelArmorGuardrail, endpoint: &str) -> ModelArmorTransport<StubTokens> {
    ModelArmorTransport {
        template_id: guardrail.template_id.clone(),
        project_id: guardrail.project_id.clone(),
        location: guardrail.location.clone(),
        api_endpoint: Some(endpoint.to_owned()),
        sanitize_error_detail: guardrail.sanitize_error_detail,
        timeout: None,
        client: client(),
        tokens: Arc::new(StubTokens),
        resolved_project: ResolvedProject::default(),
    }
}

fn params(
    endpoint: &str,
    settings: GuardrailSettings,
    sanitize_error_detail: bool,
) -> ModelArmorParams {
    ModelArmorParams {
        template_id: Some(TEMPLATE_ID.to_owned()),
        project_id: Some(PROJECT_ID.to_owned()),
        location: Some(LOCATION.to_owned()),
        credentials: None,
        api_endpoint: Some(endpoint.to_owned()),
        sanitize_error_detail: Some(sanitize_error_detail),
        async_handler: Some(client()),
        build_client: client,
        settings,
        strict_modes: true,
    }
}

// A guardrail with default_on set, so the hooks run without a requested-guardrails entry
pub(crate) fn settings() -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: Some(NAME.to_owned()),
        supported_event_hooks: None,
        event_hook: None,
        default_on: true,
        mask_request_content: false,
        mask_response_content: false,
        violation_message_template: None,
        end_session_after_n_fails: None,
        on_violation: None,
        realtime_violation_message: None,
        on_sensitive_data: None,
        sensitive_data_route_to_model: None,
        sticky_session_routing: true,
        run_in_parallel: false,
        scan_raw_request: false,
        only_scan_new_messages: false,
        timeout: None,
        switches: GuardrailSwitches::default(),
    }
}

pub(crate) fn settings_for(hooks: &[&str]) -> GuardrailSettings {
    let mut base = settings();
    base.event_hook = Some(LitellmParamsMode::Many(
        hooks.iter().map(|hook| String::from(*hook)).collect(),
    ));
    base
}

pub(crate) fn fixed_clock() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(100)
}

pub(crate) struct Armor {
    pub(crate) guardrail: ModelArmorGuardrail,
    pub(crate) transport: ModelArmorTransport<StubTokens>,
    recorded: Arc<Mutex<Vec<Recorded>>>,
    _server: Server,
}

pub(crate) async fn armor(replies: Vec<(StatusCode, String)>) -> Armor {
    armor_with(replies, settings(), false).await
}

pub(crate) async fn armor_with(
    replies: Vec<(StatusCode, String)>,
    settings: GuardrailSettings,
    sanitize_error_detail: bool,
) -> Armor {
    let (endpoint, recorded, server) = serve(replies).await;
    let guardrail = ModelArmorGuardrail::new(params(&endpoint, settings, sanitize_error_detail))
        .expect("guardrail");
    Armor {
        transport: transport(&guardrail, &endpoint),
        guardrail,
        recorded,
        _server: server,
    }
}

// A guardrail whose endpoint refused the connection, so the transport reports a request failure
pub(crate) async fn offline_armor() -> Armor {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let endpoint = format!("http://{}", listener.local_addr().expect("address"));
    drop(listener);
    let guardrail =
        ModelArmorGuardrail::new(params(&endpoint, settings(), false)).expect("guardrail");
    Armor {
        transport: transport(&guardrail, &endpoint),
        guardrail,
        recorded: Arc::new(Mutex::new(Vec::new())),
        _server: JoinSet::new(),
    }
}

impl Armor {
    pub(crate) fn hooks(&self) -> ModelArmorHooks<'_, StubTokens> {
        self.hooks_with(true)
    }

    pub(crate) fn hooks_fail_open(&self) -> ModelArmorHooks<'_, StubTokens> {
        self.hooks_with(false)
    }

    fn hooks_with(&self, fail_on_error: bool) -> ModelArmorHooks<'_, StubTokens> {
        ModelArmorHooks {
            guardrail: &self.guardrail,
            transport: &self.transport,
            fail_on_error,
            skip_unscannable_attachments: false,
            clock: fixed_clock,
        }
    }

    pub(crate) fn calls(&self) -> usize {
        self.recorded.lock().expect("recorded").len()
    }

    pub(crate) fn request(&self, index: usize) -> Recorded {
        let recorded = self.recorded.lock().expect("recorded");
        let entry = recorded
            .get(index)
            .unwrap_or_else(|| panic!("no request at {index}"));
        Recorded {
            path: entry.path.clone(),
            body: entry.body.clone(),
        }
    }

    pub(crate) fn request_text(&self, index: usize) -> String {
        sent_text(&self.request(index))
    }
}

// The prompt data object of whichever source the hook scanned
fn sent_payload(recorded: &Recorded) -> &BTreeMap<String, GuardrailValue> {
    let GuardrailValue::Object(fields) = &recorded.body else {
        panic!("no request body in {:?}", recorded.body);
    };
    let payload = fields
        .get("userPromptData")
        .or_else(|| fields.get("modelResponseData"));
    let Some(GuardrailValue::Object(payload)) = payload else {
        panic!("no prompt data in {:?}", recorded.body);
    };
    payload
}

// The text field of the sanitizeUserPrompt / sanitizeModelResponse body the hook sent
pub(crate) fn sent_text(recorded: &Recorded) -> String {
    match sent_payload(recorded).get("text") {
        Some(GuardrailValue::Text(text)) => text.clone(),
        other => panic!("no text in {other:?}"),
    }
}

// The byteData field of the sanitizeUserPrompt body a file scan sent
pub(crate) fn sent_file(recorded: &Recorded) -> String {
    let Some(GuardrailValue::Object(item)) = sent_payload(recorded).get("byteItem") else {
        panic!("no byte item in {:?}", recorded.body);
    };
    match item.get("byteData") {
        Some(GuardrailValue::Text(data)) => data.clone(),
        other => panic!("no byte data in {other:?}"),
    }
}

pub(crate) struct Recorder {
    pub(crate) invocation: GuardrailInvocation,
    pub(crate) suppressed: BTreeSet<String>,
    pub(crate) log_target: Option<GuardrailLogTarget>,
    pub(crate) model_call_details: Option<BTreeMap<String, GuardrailValue>>,
    sink: DiscardSink,
}

impl Recorder {
    pub(crate) fn new(event_type: Option<GuardrailEventHooks>) -> Self {
        Self {
            invocation: GuardrailInvocation {
                self_recorded: false,
                event_type,
                baseline: None,
                started_at: SystemTime::UNIX_EPOCH + Duration::from_millis(1_500),
            },
            suppressed: BTreeSet::new(),
            log_target: Some(log_target()),
            model_call_details: Some(logging_details()),
            sink: DiscardSink,
        }
    }

    pub(crate) fn context(&mut self) -> HookContext<'_> {
        HookContext {
            suppressed: &self.suppressed,
            invocation: &mut self.invocation,
            sink: &self.sink,
            log_target: self.log_target.as_mut(),
            model_call_details: self.model_call_details.as_mut(),
        }
    }
}

pub(crate) fn logging_details() -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([(STANDARD.to_owned(), GuardrailValue::Object(BTreeMap::new()))])
}

// The entries append_guardrail_response wrote into a model_call_details map
pub(crate) fn entries_of(details: &BTreeMap<String, GuardrailValue>) -> Vec<GuardrailValue> {
    let Some(GuardrailValue::Object(standard)) = details.get(STANDARD) else {
        return Vec::new();
    };
    match standard.get("guardrail_information") {
        None | Some(GuardrailValue::Null) => Vec::new(),
        Some(GuardrailValue::Array(entries)) => entries.clone(),
        Some(single) => vec![single.clone()],
    }
}

pub(crate) fn log_target() -> GuardrailLogTarget {
    GuardrailLogTarget {
        litellm_params: Some(BTreeMap::new()),
        model_call_params: None,
    }
}

// The entries append_guardrail_response wrote into model_call_details
pub(crate) fn standard_records(recorder: &Recorder) -> Vec<GuardrailValue> {
    let Some(details) = recorder.model_call_details.as_ref() else {
        return Vec::new();
    };
    entries_of(details)
}

pub(crate) fn value(source: &str) -> GuardrailValue {
    serde_json::from_str(source).expect("a value")
}

pub(crate) fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

pub(crate) fn data(source: &str) -> Data {
    serde_json::from_str(source).expect("request data")
}

pub(crate) fn prompt_data() -> Data {
    data(r#"{"messages":[{"role":"user","content":"hello"}]}"#)
}

pub(crate) fn prompt_with_file() -> Data {
    data(
        r#"{"messages":[{"role":"user","content":[{"type":"text","text":"hello"},{"type":"file","file":{"file_data":"aGVsbG8=","format":"application/pdf"}}]}]}"#,
    )
}

pub(crate) fn attachment_data() -> Data {
    data(
        r#"{"messages":[{"role":"user","content":[{"type":"file","file":{"file_data":"aGVsbG8=","format":"application/pdf"}}]}]}"#,
    )
}

pub(crate) fn messages(source: &str) -> Vec<Message> {
    serde_json::from_str(source).expect("messages")
}

pub(crate) fn file_settings(fail_on_error: bool) -> FileScanSettings {
    FileScanSettings {
        guardrail_name: Some(NAME.to_owned()),
        fail_on_error,
        skip_unscannable_attachments: false,
    }
}

fn inputs(texts: Option<Vec<String>>) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts,
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

pub(crate) fn texts(values: &[&str]) -> GuardrailApiInputs {
    inputs(Some(
        values.iter().map(|value| String::from(*value)).collect(),
    ))
}

pub(crate) fn no_texts() -> GuardrailApiInputs {
    inputs(None)
}

pub(crate) fn blocked<E>(error: HookError<E>) -> (u16, IndexMap<String, GuardrailValue>)
where
    E: StdError + Send + Sync + 'static,
{
    match error {
        HookError::Blocked { status, detail } => (status, detail),
        other @ (HookError::Api(_)
        | HookError::Transport(_)
        | HookError::Selection(_)
        | HookError::Field(_)
        | HookError::Response(_)
        | HookError::Metadata(_)
        | HookError::Record { .. }
        | HookError::Json { .. }) => panic!("expected a block, got {other:?}"),
    }
}

pub(crate) fn field<'value>(value: &'value GuardrailValue, name: &str) -> &'value GuardrailValue {
    match value {
        GuardrailValue::Object(fields) => fields
            .get(name)
            .unwrap_or_else(|| panic!("no field {name} in {value:?}")),
        other => panic!("expected an object, got {other:?}"),
    }
}

pub(crate) fn bucket<'data>(
    data: &'data Data,
    key: &str,
) -> &'data BTreeMap<String, GuardrailValue> {
    match data.get(key) {
        Some(GuardrailValue::Object(fields)) => fields,
        other => panic!("no {key} object in {data:?}: {other:?}"),
    }
}

// The records add_guardrail_record wrote into the request data's metadata bucket
pub(crate) fn records(data: &Data, key: &str) -> Vec<GuardrailValue> {
    match bucket(data, key).get(RECORDS) {
        None => Vec::new(),
        Some(GuardrailValue::Array(entries)) => entries.clone(),
        Some(single) => vec![single.clone()],
    }
}

pub(crate) fn model_response(source: &str) -> CallResponse {
    let model: ModelResponse = serde_json::from_str(source).expect("model response");
    CallResponse::Model(model)
}

pub(crate) fn contents(response: &CallResponse) -> Vec<String> {
    let CallResponse::Model(model) = response else {
        return Vec::new();
    };
    model
        .choices
        .iter()
        .map(|choice| match choice {
            ResponseChoice::Complete(complete) => complete.message.content.clone(),
            ResponseChoice::Streaming(streaming) => streaming.delta.content.clone(),
        })
        .collect()
}

#[derive(Clone, Default)]
struct LogSink(Arc<Mutex<Vec<u8>>>);

impl io::Write for LogSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) async fn logged<Run>(run: Run) -> String
where
    Run: Future<Output = ()>,
{
    let sink = LogSink::default();
    let writer = sink.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let guard = set_default(subscriber);
    run.await;
    drop(guard);
    let bytes = sink.0.lock().expect("log").clone();
    String::from_utf8(bytes).expect("utf8 log")
}
