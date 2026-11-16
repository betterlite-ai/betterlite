#![cfg(test)]

use std::collections::VecDeque;
use std::net::TcpListener;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{Builder, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::http::header::{ETAG, HeaderValue};
use axum::response::{IntoResponse as _, Response};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use reqwest::Client;
use secrecy::SecretString;
use tokio::net::TcpListener as AsyncListener;
use tokio::runtime::Builder as RuntimeBuilder;
use uuid::Uuid;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::GuardrailSettings;
use crate::guardrail_records::error::GuardrailSpanError;
use crate::guardrail_records::record::{GuardrailInvocationRecord, GuardrailSpanSink};
use crate::purview_dlp_evaluation::{MicrosoftPurviewDLPGuardrail, PurviewDlpGuardrailParams};
use crate::purview_graph_client::user_id_resolution::KeyIdentity;

use super::Data;
use super::context::HookContext;

pub(super) const USER_ID: &str = "user-1";
const NOW_SECONDS: u64 = 1_000_000;
const MAX_BODY_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub(super) struct Reply {
    pub(super) status: StatusCode,
    pub(super) body: String,
}

impl Reply {
    pub(super) fn ok(body: &str) -> Self {
        Self {
            status: StatusCode::OK,
            body: body.to_owned(),
        }
    }

    pub(super) fn status(code: u16, body: &str) -> Self {
        Self {
            status: StatusCode::from_u16(code).unwrap(),
            body: body.to_owned(),
        }
    }
}

pub(super) fn passing_actions() -> Reply {
    Reply::ok(r#"{"policyActions":[]}"#)
}

pub(super) fn blocking_actions() -> Reply {
    Reply::ok(
        r##"{"policyActions":[{"@odata.type":"#microsoft.graph.restrictAccessAction","action":"restrictAccess","restrictionAction":"block"}]}"##,
    )
}

pub(super) struct Script {
    token: Reply,
    scopes: Reply,
    process: Vec<Reply>,
}

impl Script {
    pub(super) fn new(process: Vec<Reply>) -> Self {
        Self {
            token: Reply::ok(r#"{"access_token":"tok","expires_in":3600}"#),
            scopes: Reply::ok(r#"{"value":[]}"#),
            process,
        }
    }

    pub(super) fn passing() -> Self {
        Self::new(vec![passing_actions()])
    }
}

#[derive(Clone)]
pub(super) struct SeenRequest {
    pub(super) path: String,
    pub(super) body: String,
}

struct MockState {
    token: Reply,
    scopes: Reply,
    process: Mutex<VecDeque<Reply>>,
    seen: Arc<Mutex<Vec<SeenRequest>>>,
}

pub(super) struct GraphMock {
    pub(super) base: String,
    seen: Arc<Mutex<Vec<SeenRequest>>>,
    _server: JoinHandle<()>,
}

impl GraphMock {
    pub(super) fn seen(&self) -> Vec<SeenRequest> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn process_calls(&self) -> usize {
        self.seen()
            .iter()
            .filter(|request| request.path.ends_with("/processContent"))
            .count()
    }

    pub(super) fn scanned(&self) -> Vec<Scan> {
        self.seen()
            .iter()
            .filter(|request| request.path.ends_with("/processContent"))
            .map(|request| {
                let body: ProcessBody = serde_json::from_str(&request.body).unwrap();
                let entry = body.content_to_process.content_entries.first().unwrap();
                Scan {
                    activity: body.content_to_process.activity_metadata.activity.clone(),
                    data: entry.content.data.clone(),
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub(super) struct Scan {
    pub(super) activity: String,
    pub(super) data: String,
}

#[derive(serde::Deserialize)]
struct ProcessBody {
    #[serde(rename = "contentToProcess")]
    content_to_process: ContentToProcess,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContentToProcess {
    content_entries: Vec<ContentEntry>,
    activity_metadata: ActivityMetadata,
}

#[derive(serde::Deserialize)]
struct ContentEntry {
    content: TextContent,
}

#[derive(serde::Deserialize)]
struct TextContent {
    data: String,
}

#[derive(serde::Deserialize)]
struct ActivityMetadata {
    activity: String,
}

async fn serve(State(state): State<Arc<MockState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, MAX_BODY_BYTES).await.unwrap();
    let path = parts.uri.path().to_owned();
    state
        .seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(SeenRequest {
            path: path.clone(),
            body: String::from_utf8(bytes.to_vec()).unwrap(),
        });
    let reply = if path.ends_with("/processContent") {
        state
            .process
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Reply::status(500, "{}"))
    } else if path.ends_with("/protectionScopes/compute") {
        state.scopes.clone()
    } else {
        state.token.clone()
    };
    let mut response = (reply.status, reply.body).into_response();
    let _previous = response
        .headers_mut()
        .insert(ETAG, HeaderValue::from_static("\"etag-1\""));
    response
}

pub(super) fn start(script: Script) -> GraphMock {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let state = Arc::new(MockState {
        token: script.token,
        scopes: script.scopes,
        process: Mutex::new(VecDeque::from(script.process)),
        seen: Arc::clone(&seen),
    });
    let router = Router::new().fallback(serve).with_state(state);
    let server = Builder::new()
        .name("purview-graph-mock".to_owned())
        .spawn(move || {
            let runtime = RuntimeBuilder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = AsyncListener::from_std(listener).unwrap();
                axum::serve(listener, router).await.unwrap();
            });
        })
        .unwrap();
    GraphMock {
        base,
        seen,
        _server: server,
    }
}

pub(super) fn fixed_clock() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(NOW_SECONDS))
        .unwrap()
}

pub(super) fn guardrail_at(base: &str) -> MicrosoftPurviewDLPGuardrail {
    let params = PurviewDlpGuardrailParams {
        guardrail_name: "purview-test".to_owned(),
        tenant_id: "tenant-1".to_owned(),
        client_id: "client-1".to_owned(),
        client_secret: SecretString::from("secret-1"),
        purview_app_name: None,
        user_id_field: None,
        timeout: Some(Duration::from_secs(5)),
    };
    let mut guardrail = MicrosoftPurviewDLPGuardrail::new(params, Client::default());
    guardrail.base.graph_api_base = base.to_owned();
    guardrail.base.login_authority = base.to_owned();
    guardrail.base.clock = fixed_clock;
    guardrail.base.new_id = Uuid::nil;
    guardrail
}

pub(super) fn record_base() -> GuardrailBase {
    GuardrailBase {
        settings: GuardrailSettings::default(),
        mode: None,
    }
}

pub(super) fn redactor() -> Arc<Redactor> {
    Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap())
}

pub(super) fn cache() -> DualLocalCache {
    let diagnostics = Diagnostics {
        redactor: redactor(),
        emit: Arc::new(|_event| {}),
    };
    DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        diagnostics,
    )
}

pub(super) fn data(json: &str) -> Data {
    serde_json::from_str(json).unwrap()
}

pub(super) fn value(json: &str) -> GuardrailValue {
    serde_json::from_str(json).unwrap()
}

pub(super) fn values(json: &str) -> Vec<GuardrailValue> {
    serde_json::from_str(json).unwrap()
}

pub(super) fn trusted_identity() -> KeyIdentity<'static> {
    KeyIdentity {
        user_id: Some(USER_ID),
        end_user_id: None,
    }
}

pub(super) fn untrusted_identity() -> KeyIdentity<'static> {
    KeyIdentity {
        user_id: None,
        end_user_id: Some("end-user-1"),
    }
}

#[derive(Default)]
pub(super) struct TestSink {
    pub(super) records: Mutex<Vec<GuardrailInvocationRecord>>,
}

impl TestSink {
    pub(super) fn records(&self) -> Vec<GuardrailInvocationRecord> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl GuardrailSpanSink for TestSink {
    fn emit(&self, record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record.clone());
        Ok(())
    }
}

pub(super) fn hook_context<'ctx>(
    identity: KeyIdentity<'ctx>,
    cache: &Arc<DualLocalCache>,
    redactor: &Arc<Redactor>,
    base: &'ctx GuardrailBase,
    sink: &'ctx TestSink,
) -> HookContext<'ctx> {
    HookContext {
        identity,
        cache: Arc::clone(cache),
        redactor: Arc::clone(redactor),
        record_base: base,
        invocation: None,
        sink,
        log_target: None,
    }
}

pub(super) struct Harness {
    pub(super) data: Data,
    pub(super) base: GuardrailBase,
    pub(super) sink: TestSink,
    pub(super) cache: Arc<DualLocalCache>,
    pub(super) redactor: Arc<Redactor>,
}

impl Harness {
    pub(super) fn new(data: Data) -> Self {
        Self {
            data,
            base: record_base(),
            sink: TestSink::default(),
            cache: Arc::new(cache()),
            redactor: redactor(),
        }
    }
}
