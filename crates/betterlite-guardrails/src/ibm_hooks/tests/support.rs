#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::post;
use betterlite_config::tls::Verification;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::ibm_detector_client::client::IbmDetectorClient;
use crate::ibm_detector_client::config::IbmDetectorConfig;
use crate::ibm_hooks::IbmGuardrail;
use crate::ibm_hooks::config::EventHook;
use crate::ibm_hooks::data::PromptData;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    DetectorServer,
    Orchestrator,
}

pub(crate) fn config(mode: Mode, api_url: &str, block_on_detection: bool) -> IbmDetectorConfig {
    IbmDetectorConfig {
        guardrail_name: "ibm".to_owned(),
        auth_token: SecretString::from("token"),
        base_url: "https://detector.test".to_owned(),
        api_url: api_url.to_owned(),
        detector_id: "jailbreak".to_owned(),
        is_detector_server: mode == Mode::DetectorServer,
        detector_params: BTreeMap::new(),
        extra_headers: BTreeMap::new(),
        score_threshold: None,
        block_on_detection,
        ssl_verification: Verification::Enabled,
        timeout: None,
    }
}

pub(crate) fn guardrail(
    config: IbmDetectorConfig,
    event_hooks: Option<Vec<EventHook>>,
) -> IbmGuardrail {
    IbmGuardrail {
        client: IbmDetectorClient::new(config).expect("client"),
        event_hooks,
    }
}

pub(crate) fn data(raw: &str) -> PromptData {
    serde_json::from_str(raw).expect("prompt data")
}

#[derive(serde::Deserialize)]
pub(crate) struct DetectorServerBody {
    pub(crate) contents: Vec<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct OrchestratorBody {
    pub(crate) content: String,
}

pub(crate) async fn capturing_server(
    path: &'static str,
    status: StatusCode,
    body: &str,
    capacity: usize,
) -> (String, mpsc::Receiver<String>, JoinSet<io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let (capture_sender, receiver) = mpsc::channel(capacity);
    let response_body = body.to_owned();
    let router = Router::new().route(
        path,
        post(move |request_body: String| {
            let sender = capture_sender.clone();
            let reply = response_body.clone();
            async move {
                drop(sender.send(request_body).await);
                (status, reply)
            }
        }),
    );
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, receiver, server)
}
