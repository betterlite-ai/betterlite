use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::future::IntoFuture as _;
use std::io;
use std::path::PathBuf;
use std::process;
use std::str::from_utf8;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::panw_prisma_airs::{
    Configured, ContentMaskSettings, FallbackAction, MaskingSettings,
};
use betterlite_config::tls::{TlsPolicy, Verification};
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinSet;
use tracing::subscriber::{DefaultGuard, set_default};
use uuid::Uuid;

use crate::guardrail_message_scope::StreamItem;
use crate::guardrail_metadata::ScanEntry;
use crate::guardrail_selection::error::GuardrailSelectionError;
use crate::guardrail_selection::gate::GuardrailSelection;
use crate::panw_prisma_airs::audit::GuardrailAudit;
use crate::panw_prisma_airs::construct::PanwPrismaAirs;
use crate::panw_prisma_airs::error_detail::GuardrailHttpError;
use crate::panw_prisma_airs::hooks::context::HookContext;
use crate::panw_prisma_airs::hooks::data::Data;
use crate::panw_prisma_airs::hooks::post_call::post_call_hook;
use crate::panw_prisma_airs::hooks::response::CallResponse;
use crate::panw_prisma_airs::hooks::streaming::hook::streaming_iterator_hook;
use crate::panw_prisma_airs::hooks::streaming::{
    StreamAssembler, StreamAssemblyError, StreamChunk, StreamingResponse,
};
use crate::prisma_airs_masking::{MaskContent, MaskedResponseMessage, Response};

pub(super) const SCAN_PATH: &str = "/v1/scan/sync/request";
pub(super) const APPLIED: &str = "applied_guardrails";
pub(super) const RECORDS: &str = "standard_logging_guardrail_information";

pub(super) const NO_MASKING: MaskingSettings = MaskingSettings {
    mask_on_block: false,
    content: ContentMaskSettings {
        request_content: false,
        response_content: false,
    },
};

pub(super) const REQUEST_MASKING: MaskingSettings = MaskingSettings {
    mask_on_block: false,
    content: ContentMaskSettings {
        request_content: true,
        response_content: false,
    },
};

pub(super) const RESPONSE_MASKING: MaskingSettings = MaskingSettings {
    mask_on_block: false,
    content: ContentMaskSettings {
        request_content: false,
        response_content: true,
    },
};

pub(super) fn settings(
    api_base: &str,
    masking: MaskingSettings,
    fallback: FallbackAction,
) -> Configured {
    Configured {
        guardrail_name: "panw".to_owned(),
        api_key: SecretString::from("airs-key".to_owned()),
        api_base: api_base.to_owned(),
        profile_name: Some("config-profile".to_owned()),
        app_name: "LiteLLM".to_owned(),
        masking,
        fallback_on_error: fallback,
        timeout: 5.0,
        violation_message_template: None,
        default_on: true,
        latest_role_message_only: None,
    }
}

fn tls() -> TlsPolicy {
    TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite-tests".to_owned(),
    }
}

pub(super) fn new_id() -> Uuid {
    Uuid::from_u128(7)
}

pub(super) fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(12))
        .expect("time in range")
}

pub(super) fn guardrail(configured: Configured) -> PanwPrismaAirs {
    let mut built = PanwPrismaAirs::new(configured, &tls()).expect("client builds");
    built.now = now;
    built
}

pub(super) fn blocking(api_base: &str) -> PanwPrismaAirs {
    guardrail(settings(api_base, NO_MASKING, FallbackAction::Block))
}

pub(super) fn allowing(api_base: &str) -> PanwPrismaAirs {
    guardrail(settings(api_base, NO_MASKING, FallbackAction::Allow))
}

pub(super) fn masking_requests(api_base: &str) -> PanwPrismaAirs {
    guardrail(settings(api_base, REQUEST_MASKING, FallbackAction::Block))
}

pub(super) fn masking_responses(api_base: &str) -> PanwPrismaAirs {
    guardrail(settings(api_base, RESPONSE_MASKING, FallbackAction::Block))
}

// The hook only ever asks the selection about the events a test allows.
pub(super) struct FakeSelection {
    pub(super) events: Vec<GuardrailEventHooks>,
}

impl GuardrailSelection for FakeSelection {
    fn should_run_guardrail(
        &self,
        _data: &BTreeMap<String, GuardrailValue>,
        event_type: GuardrailEventHooks,
        _suppressed: &BTreeSet<String>,
    ) -> Result<bool, GuardrailSelectionError> {
        Ok(self.events.contains(&event_type))
    }

    fn event_hook_is_event_type(&self, event_type: GuardrailEventHooks) -> bool {
        self.events.contains(&event_type)
    }

    fn is_requested_guardrail(&self, _requested: &[GuardrailValue]) -> bool {
        true
    }

    fn dynamic_request_body_params(
        &self,
        _data: &BTreeMap<String, GuardrailValue>,
    ) -> Result<BTreeMap<String, GuardrailValue>, GuardrailSelectionError> {
        Ok(BTreeMap::new())
    }
}

pub(super) fn context<'context>(
    selection: &'context FakeSelection,
    suppressed: &'context BTreeSet<String>,
) -> HookContext<'context> {
    HookContext {
        selection,
        suppressed,
        new_id,
    }
}

pub(super) fn json(source: &str) -> GuardrailValue {
    serde_json::from_str(source).expect("json fixture")
}

pub(super) fn data(source: &str) -> Data {
    serde_json::from_str(source).expect("request data")
}

pub(super) fn response(source: &str) -> Response {
    serde_json::from_str(source).expect("model response")
}

pub(super) fn object_of(value: &GuardrailValue) -> &BTreeMap<String, GuardrailValue> {
    let GuardrailValue::Object(fields) = value else {
        panic!("not an object");
    };
    fields
}

pub(super) fn array_of(value: &GuardrailValue) -> &[GuardrailValue] {
    let GuardrailValue::Array(items) = value else {
        panic!("not an array");
    };
    items
}

pub(super) fn text_of(value: &GuardrailValue) -> &str {
    let GuardrailValue::Text(text) = value else {
        panic!("not text");
    };
    text
}

pub(super) fn field<'value>(value: &'value GuardrailValue, key: &str) -> &'value GuardrailValue {
    object_of(value).get(key).expect("field")
}

pub(super) fn message_at(response: &Response, index: usize) -> &MaskedResponseMessage {
    let choices = response.choices.as_ref().expect("choices");
    choices
        .get(index)
        .expect("choice")
        .message
        .as_ref()
        .expect("message")
}

pub(super) fn text_content(message: &MaskedResponseMessage) -> &str {
    match message.content.as_ref() {
        Some(MaskContent::Text(text)) => text,
        _ => panic!("text content"),
    }
}

pub(super) fn scanned_text(body: &GuardrailValue, key: &str) -> String {
    let contents = array_of(field(body, "contents"));
    let content = contents.first().expect("content");
    text_of(field(content, key)).to_owned()
}

// The bucket the hooks write when litellm_metadata is present, else metadata.
pub(super) fn bucket(data: &Data) -> &BTreeMap<String, GuardrailValue> {
    match data
        .get("litellm_metadata")
        .or_else(|| data.get("metadata"))
    {
        Some(GuardrailValue::Object(fields)) => fields,
        _ => panic!("no metadata bucket"),
    }
}

pub(super) fn bucket_field<'data>(data: &'data Data, key: &str) -> Option<&'data GuardrailValue> {
    bucket(data).get(key)
}

pub(super) fn records(data: &Data) -> &[GuardrailValue] {
    match bucket_field(data, RECORDS) {
        Some(GuardrailValue::Array(entries)) => entries,
        _ => panic!("no records"),
    }
}

pub(super) fn names_in(data: &Data, key: &str) -> Vec<String> {
    let Some(GuardrailValue::Array(items)) = bucket_field(data, key) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for item in items {
        if let GuardrailValue::Text(name) = item {
            names.push(name.clone());
        }
    }
    names
}

pub(super) fn scan_entries(data: &Data) -> Vec<ScanEntry> {
    let Some(GuardrailValue::Array(items)) = bucket_field(data, "guardrail_scan_metadata") else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for item in items {
        let encoded = serde_json::to_value(item).expect("scan entry json");
        entries.push(serde_json::from_value::<ScanEntry>(encoded).expect("scan entry"));
    }
    entries
}

pub(super) fn frame_json(item: &StreamItem) -> GuardrailValue {
    let StreamItem::Bytes(bytes) = item else {
        panic!("frame is not bytes");
    };
    let text = from_utf8(bytes).expect("frame utf8");
    let prefixed = text.strip_prefix("data: ").expect("sse prefix");
    let payload = prefixed.strip_suffix("\n\n").expect("sse suffix");
    serde_json::from_str(payload).expect("frame json")
}

#[derive(Clone)]
struct Reply {
    status: StatusCode,
    body: Vec<u8>,
    bodies: Arc<AsyncMutex<Vec<String>>>,
}

pub(super) struct Server {
    pub(super) base: String,
    bodies: Arc<AsyncMutex<Vec<String>>>,
    _tasks: JoinSet<io::Result<()>>,
}

impl Server {
    pub(super) async fn body_texts(&self) -> Vec<String> {
        self.bodies.lock().await.clone()
    }
}

pub(super) async fn serve(status: StatusCode, body: &str) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let bodies = Arc::new(AsyncMutex::new(Vec::new()));
    let reply = Reply {
        status,
        body: body.as_bytes().to_vec(),
        bodies: Arc::clone(&bodies),
    };
    let router = Router::new().route(SCAN_PATH, post(scan).with_state(reply));
    let mut tasks = JoinSet::new();
    tasks.spawn(axum::serve(listener, router).into_future());
    Server {
        base,
        bodies,
        _tasks: tasks,
    }
}

async fn scan(State(reply): State<Reply>, body: String) -> (StatusCode, Vec<u8>) {
    reply.bodies.lock().await.push(body);
    (reply.status, reply.body)
}

pub(super) async fn only_scan_body(server: &Server) -> GuardrailValue {
    let bodies = server.body_texts().await;
    json(bodies.first().expect("one scan"))
}

// Runs the streaming hook with the post_call selection its tests use and returns its items.
pub(super) async fn run_streaming(
    guardrail: &PanwPrismaAirs,
    data: &mut Data,
    chunks: Vec<StreamChunk>,
    assembler: &(dyn StreamAssembler + Sync),
) -> Vec<StreamItem> {
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PostCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    streaming_iterator_hook(
        guardrail,
        data,
        StreamingResponse { chunks, assembler },
        &mut audit,
        &hook_context,
    )
    .await
    .expect("streaming hook")
}

// Runs the post-call hook with the post_call selection its tests use.
pub(super) async fn run_post_call(
    guardrail: &PanwPrismaAirs,
    data: &mut Data,
    response: &mut Response,
) -> Result<(), GuardrailHttpError> {
    let selection = FakeSelection {
        events: vec![GuardrailEventHooks::PostCall],
    };
    let suppressed = BTreeSet::new();
    let hook_context = context(&selection, &suppressed);
    let mut audit = GuardrailAudit::default();
    post_call_hook(
        guardrail,
        data,
        CallResponse::Model(response),
        &mut audit,
        &hook_context,
    )
    .await
}

pub(super) struct LogCapture {
    path: PathBuf,
    guard: DefaultGuard,
}

impl LogCapture {
    pub(super) fn start(name: &str, level: tracing::Level) -> Self {
        let path = env::temp_dir().join(format!(
            "betterlite-panw-hooks-{name}-{}.log",
            process::id()
        ));
        let writer = Arc::new(fs::File::create(&path).expect("log file"));
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(level)
            .with_writer(writer)
            .finish();
        let guard = set_default(subscriber);
        Self { path, guard }
    }

    pub(super) fn finish(self) -> String {
        drop(self.guard);
        let text = fs::read_to_string(&self.path).expect("log contents");
        fs::remove_file(&self.path).expect("remove log");
        text
    }
}

// Stands in for stream_chunk_builder and MockResponseIterator until U2 wires them in.
pub(super) struct FakeAssembler {
    pub(super) assembled: Option<String>,
    assembled_chunks: AtomicUsize,
    reassembled: AtomicUsize,
}

impl FakeAssembler {
    pub(super) fn new(assembled: Option<&str>) -> Self {
        Self {
            assembled: assembled.map(str::to_owned),
            assembled_chunks: AtomicUsize::new(0),
            reassembled: AtomicUsize::new(0),
        }
    }

    pub(super) fn assembled_chunk_count(&self) -> usize {
        self.assembled_chunks.load(Ordering::SeqCst)
    }

    pub(super) fn reassembled_count(&self) -> usize {
        self.reassembled.load(Ordering::SeqCst)
    }
}

impl StreamAssembler for FakeAssembler {
    fn assemble(&self, chunks: &[GuardrailValue]) -> Result<Option<Response>, StreamAssemblyError> {
        self.assembled_chunks.store(chunks.len(), Ordering::SeqCst);
        self.assembled
            .as_deref()
            .map_or(Ok(None), |source| Ok(Some(response(source))))
    }

    fn reassemble(&self, response: &Response) -> Result<Vec<StreamChunk>, StreamAssemblyError> {
        self.reassembled.fetch_add(1, Ordering::SeqCst);
        let mut rebuilt = Vec::new();
        for choice in response.choices.iter().flatten() {
            let Some(message) = choice.message.as_ref() else {
                continue;
            };
            let Some(MaskContent::Text(text)) = message.content.as_ref() else {
                continue;
            };
            rebuilt.push(StreamChunk::Bytes(format!("data: {text}\n\n").into_bytes()));
        }
        Ok(rebuilt)
    }
}
