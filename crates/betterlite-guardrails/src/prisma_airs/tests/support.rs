#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use betterlite_config::sections::panw_prisma_airs::{
    Configured, ContentMaskSettings, FallbackAction, MaskingSettings,
};
use betterlite_config::tls::{TlsPolicy, Verification};
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::panw_prisma_airs::audit::{Direction, GuardrailAudit};
use crate::panw_prisma_airs::construct::PanwPrismaAirs;
use crate::panw_prisma_airs::tool_scan::ToolCall;
use crate::prisma_airs::error::UnifiedError;
use crate::prisma_airs::inputs::{Inputs, StructuredMessage};
use crate::prisma_airs::request_data::{LoggingObject, RequestData, ResponsesInput};
use crate::prisma_airs::unified::{ScanContext, apply_guardrail};
use crate::prisma_airs_masking::mcp::McpServer;

pub(super) const SCAN_PATH: &str = "/v1/scan/sync/request";

pub(super) fn settings(api_base: &str) -> Configured {
    Configured {
        guardrail_name: "panw".to_owned(),
        api_key: SecretString::from("airs-key".to_owned()),
        api_base: api_base.to_owned(),
        profile_name: Some("config-profile".to_owned()),
        app_name: "LiteLLM".to_owned(),
        masking: MaskingSettings {
            mask_on_block: false,
            content: ContentMaskSettings {
                request_content: false,
                response_content: false,
            },
        },
        fallback_on_error: FallbackAction::Block,
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

fn twelve_seconds_in() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(12))
        .expect("time in range")
}

pub(super) fn build_guardrail(settings: Configured) -> PanwPrismaAirs {
    let mut built = PanwPrismaAirs::new(settings, &tls()).expect("client builds");
    built.now = twelve_seconds_in;
    built
}

pub(super) fn fixed_id() -> Uuid {
    Uuid::nil()
}

pub(super) fn request_data() -> RequestData {
    RequestData {
        litellm_call_id: None,
        model: None,
        user: None,
        litellm_trace_id: None,
        metadata: None,
        mcp_tool_name: None,
        name: None,
        arguments: None,
        mcp_arguments: None,
        mcp_server_name: None,
        server_name: None,
        server_id: None,
        proxy_server_request: None,
        instructions: None,
        input: None,
    }
}

pub(super) fn inputs(texts: &[&str]) -> Inputs {
    Inputs {
        texts: texts.iter().map(|text| (*text).to_owned()).collect(),
        structured_messages: None,
        tool_calls: None,
        model: None,
    }
}

pub(super) fn messages(json: &str) -> Vec<StructuredMessage> {
    serde_json::from_str(json).expect("messages parse")
}

pub(super) fn texts(values: &[&str]) -> Vec<String> {
    values.iter().map(|text| (*text).to_owned()).collect()
}

pub(super) fn responses_input(json: &str) -> ResponsesInput {
    serde_json::from_str(json).expect("responses input parses")
}

pub(super) fn logging() -> LoggingObject {
    LoggingObject {
        litellm_call_id: None,
        model: None,
        call_type: None,
        model_call_details: None,
    }
}

pub(super) fn tool_calls(json: &str) -> Vec<ToolCall> {
    serde_json::from_str(json).expect("tool calls parse")
}

pub(super) fn prompt_json(text: &str) -> String {
    let encoded = serde_json::to_string(text).expect("text encodes");
    format!(r#""contents":[{{"prompt":{encoded}}}]"#)
}

pub(super) fn response_json(text: &str) -> String {
    let encoded = serde_json::to_string(text).expect("text encodes");
    format!(r#""contents":[{{"response":{encoded}}}]"#)
}

#[derive(Clone)]
struct Reply {
    status: StatusCode,
    body: Vec<u8>,
    captured: Arc<Mutex<Vec<String>>>,
}

pub(super) struct Server {
    pub(super) base: String,
    captured: Arc<Mutex<Vec<String>>>,
    _tasks: JoinSet<io::Result<()>>,
}

impl Server {
    pub(super) async fn bodies(&self) -> Vec<String> {
        self.captured.lock().await.clone()
    }
}

pub(super) async fn serve_status(status: StatusCode, body: &str) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let reply = Reply {
        status,
        body: body.as_bytes().to_vec(),
        captured: Arc::clone(&captured),
    };
    let router = Router::new().route(SCAN_PATH, post(scan).with_state(reply));
    let mut tasks = JoinSet::new();
    tasks.spawn(axum::serve(listener, router).into_future());
    Server {
        base,
        captured,
        _tasks: tasks,
    }
}

async fn scan(
    State(reply): State<Reply>,
    _headers: HeaderMap,
    body: String,
) -> (StatusCode, Vec<u8>) {
    reply.captured.lock().await.push(body);
    (reply.status, reply.body)
}

pub(super) struct Run {
    pub(super) reply: String,
    pub(super) status: StatusCode,
    pub(super) direction: Direction,
    pub(super) mask_request: bool,
    pub(super) mask_response: bool,
    pub(super) latest_only: Option<bool>,
    pub(super) fallback: FallbackAction,
    pub(super) servers: BTreeMap<String, McpServer>,
    pub(super) tool_names: BTreeMap<String, String>,
}

impl Run {
    // Test defaults mirror the guardrail config defaults, so a scenario only names what it changes.
    pub(super) fn new(reply: &str) -> Self {
        let defaults = settings("");
        Self {
            reply: reply.to_owned(),
            status: StatusCode::OK,
            direction: Direction::Request,
            mask_request: defaults.masking.content.request_content,
            mask_response: defaults.masking.content.response_content,
            latest_only: defaults.latest_role_message_only,
            fallback: defaults.fallback_on_error,
            servers: BTreeMap::new(),
            tool_names: BTreeMap::new(),
        }
    }
}

pub(super) struct Outcome {
    pub(super) result: Result<(), UnifiedError>,
    pub(super) inputs: Inputs,
    pub(super) request_data: RequestData,
    pub(super) audit: GuardrailAudit,
    pub(super) bodies: Vec<String>,
}

pub(super) async fn execute(
    run: Run,
    mut inputs: Inputs,
    mut data: RequestData,
    logging: Option<&LoggingObject>,
) -> Outcome {
    let server = serve_status(run.status, &run.reply).await;
    let mut config = settings(&server.base);
    config.masking.content.request_content = run.mask_request;
    config.masking.content.response_content = run.mask_response;
    config.fallback_on_error = run.fallback;
    config.latest_role_message_only = run.latest_only;
    let guardrail = build_guardrail(config);
    let mut audit = GuardrailAudit::default();
    let mut context = ScanContext {
        request_data: &mut data,
        input_type: run.direction,
        logging_obj: logging,
        servers_by_id: &run.servers,
        names_by_tool: &run.tool_names,
        new_id: fixed_id,
    };
    let result = apply_guardrail(&guardrail, &mut inputs, &mut context, &mut audit).await;
    let bodies = server.bodies().await;
    Outcome {
        result,
        inputs,
        request_data: data,
        audit,
        bodies,
    }
}
