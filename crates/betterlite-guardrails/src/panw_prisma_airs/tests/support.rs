use std::future::IntoFuture as _;
use std::io;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::routing::{get, post};
use betterlite_config::sections::panw_prisma_airs::{
    Configured, ContentMaskSettings, FallbackAction, MaskingSettings,
};
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::panw_prisma_airs::construct::PanwPrismaAirs;
use crate::panw_prisma_airs::metadata::{RequestData, ScanMetadata, prepare_metadata_from_request};

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

fn twelve_seconds_in() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(12))
        .expect("time in range")
}

pub(super) fn guardrail(settings: Configured) -> PanwPrismaAirs {
    let mut built = PanwPrismaAirs::new(settings).expect("client builds");
    built.now = twelve_seconds_in;
    built
}

pub(super) fn metadata() -> ScanMetadata {
    prepare_metadata_from_request(&RequestData::default())
}

#[derive(Debug, Clone)]
pub(super) struct Captured {
    pub(super) token: Option<String>,
    pub(super) content_type: Option<String>,
    pub(super) body: String,
}

#[derive(Clone)]
struct Reply {
    status: StatusCode,
    body: Vec<u8>,
    delay: Duration,
    captured: Arc<Mutex<Vec<Captured>>>,
}

pub(super) struct Server {
    pub(super) base: String,
    pub(super) captured: Arc<Mutex<Vec<Captured>>>,
    pub(super) redirect_hits: Arc<Mutex<Vec<Captured>>>,
    _tasks: JoinSet<io::Result<()>>,
}

pub(super) async fn serve(status: StatusCode, body: &[u8]) -> Server {
    serve_after(status, body, Duration::ZERO).await
}

pub(super) async fn serve_after(status: StatusCode, body: &[u8], delay: Duration) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let captured = Arc::new(Mutex::new(Vec::new()));
    let redirect_hits = Arc::new(Mutex::new(Vec::new()));
    let reply = Reply {
        status,
        body: body.to_vec(),
        delay,
        captured: Arc::clone(&captured),
    };
    let elsewhere = Reply {
        status: StatusCode::OK,
        body: Vec::new(),
        delay: Duration::ZERO,
        captured: Arc::clone(&redirect_hits),
    };
    let router = Router::new()
        .route(SCAN_PATH, post(scan).with_state(reply))
        .route("/elsewhere", get(scan).with_state(elsewhere));
    let mut tasks = JoinSet::new();
    tasks.spawn(axum::serve(listener, router).into_future());
    Server {
        base,
        captured,
        redirect_hits,
        _tasks: tasks,
    }
}

async fn scan(
    State(reply): State<Reply>,
    headers: HeaderMap,
    body: String,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let text = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    reply.captured.lock().await.push(Captured {
        token: text("x-pan-token"),
        content_type: text(header::CONTENT_TYPE.as_str()),
        body,
    });
    sleep(reply.delay).await;
    let mut out = HeaderMap::new();
    if reply.status.is_redirection() {
        out.insert(header::LOCATION, "/elsewhere".parse().expect("location"));
    }
    (reply.status, out, reply.body)
}
