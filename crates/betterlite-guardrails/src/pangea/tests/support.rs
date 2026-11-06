use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io::Error as IoError;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap as AxumHeaderMap, StatusCode as AxumStatusCode, Uri};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::generic_guardrail_contracts::GuardrailMessage;
use crate::javelin::AppliedGuardrailsHeader;
use crate::pangea::{PangeaGuardrail, PangeaGuardrailConfigModel, RequestData};

pub(crate) const GUARDRAIL_NAME: &str = "pangea-ai-guard";

#[derive(Debug, Default)]
pub(crate) struct Captured {
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Vec<u8>,
}

pub(crate) struct Server {
    pub(crate) base: String,
    captured: Arc<Mutex<Captured>>,
    _tasks: JoinSet<Result<(), IoError>>,
}

impl Server {
    pub(crate) fn captured(&self) -> Captured {
        let captured = self.captured.lock().unwrap();
        Captured {
            path: captured.path.clone(),
            headers: captured.headers.clone(),
            body: captured.body.clone(),
        }
    }
}

pub(crate) async fn serve(status: StatusCode, body: String) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let captured = Arc::new(Mutex::new(Captured::default()));
    let captured_for_handler = Arc::clone(&captured);
    let answer = move |uri: Uri, headers: AxumHeaderMap, bytes: Bytes| {
        let captured_for_body = Arc::clone(&captured_for_handler);
        async move {
            let mut recorded = captured_for_body.lock().unwrap();
            recorded.path = uri.path().to_owned();
            recorded.headers = headers
                .iter()
                .filter_map(|(name, header_value)| {
                    header_value
                        .to_str()
                        .ok()
                        .map(|text| (name.as_str().to_owned(), text.to_owned()))
                })
                .collect();
            recorded.body = bytes.to_vec();
            drop(recorded);
            (AxumStatusCode::from_u16(status.as_u16()).unwrap(), body)
        }
    };
    let router = Router::new().fallback(answer);
    let mut tasks: JoinSet<Result<(), IoError>> = JoinSet::new();
    tasks.spawn(axum::serve(listener, router).into_future());
    Server {
        base,
        captured,
        _tasks: tasks,
    }
}

pub(crate) async fn serve_hanging() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let answer = || async {
        sleep(Duration::from_secs(30)).await;
        (AxumStatusCode::OK, String::new())
    };
    let router = Router::new().fallback(answer);
    let mut tasks: JoinSet<Result<(), IoError>> = JoinSet::new();
    tasks.spawn(axum::serve(listener, router).into_future());
    Server {
        base,
        captured: Arc::new(Mutex::new(Captured::default())),
        _tasks: tasks,
    }
}

pub(crate) fn guardrail(base: &str) -> PangeaGuardrail {
    PangeaGuardrail {
        guardrail_name: GUARDRAIL_NAME.to_owned(),
        api_key: SecretString::from("test-key".to_owned()),
        api_base: base.to_owned(),
        input_recipe: None,
        output_recipe: None,
        client: reqwest::Client::default(),
    }
}

pub(crate) fn config() -> PangeaGuardrailConfigModel {
    PangeaGuardrailConfigModel::default()
}

pub(crate) fn user(content: &str) -> GuardrailMessage {
    GuardrailMessage {
        role: "user".to_owned(),
        extra: BTreeMap::from([(
            "content".to_owned(),
            GuardrailValue::Text(content.to_owned()),
        )]),
    }
}

pub(crate) fn data_with_messages(messages: Vec<GuardrailMessage>) -> RequestData {
    RequestData {
        messages: Some(messages),
        prompt: None,
        tools: None,
    }
}

#[derive(Debug, Default)]
pub(crate) struct Recorder {
    pub(crate) names: Vec<String>,
}

impl AppliedGuardrailsHeader for Recorder {
    fn record_applied_guardrail(&mut self, guardrail_name: &str) {
        self.names.push(guardrail_name.to_owned());
    }
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct CapturedPayload {
    pub(crate) debug: bool,
    pub(crate) input: CapturedInput,
    pub(crate) event_type: String,
    #[serde(default)]
    pub(crate) recipe: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct CapturedInput {
    #[serde(default)]
    pub(crate) messages: Option<Vec<GuardrailMessage>>,
    #[serde(default)]
    pub(crate) tools: Option<GuardrailValue>,
    #[serde(default)]
    pub(crate) choices: Option<Vec<GuardrailValue>>,
}

pub(crate) fn payload(captured: &Captured) -> CapturedPayload {
    serde_json::from_slice(&captured.body).unwrap()
}
