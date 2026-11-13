use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::singulr::RequestData;
use crate::singulr::SingulrGuardrail;
use crate::singulr::init::SingulrArgs;

pub(crate) const API_KEY: &str = "singulr-token";
pub(crate) const APPLICATION_ID: &str = "application-1";
pub(crate) const GUARDRAIL_ID: &str = "guardrail-1";

pub(crate) fn args() -> SingulrArgs {
    SingulrArgs {
        name: "singulr-check".to_owned(),
        singulr_api_key: Some(SecretString::from(API_KEY)),
        singulr_api_base: None,
        singulr_application_id: Some(APPLICATION_ID.to_owned()),
        singulr_guardrail_id: Some(GUARDRAIL_ID.to_owned()),
        block_on_error: Some(true),
        timeout: Some(5.0),
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: false,
    }
}

pub(crate) fn args_for(base: &str) -> SingulrArgs {
    let mut built = args();
    built.singulr_api_base = Some(base.to_owned());
    built
}

pub(crate) fn build(built: &SingulrArgs) -> SingulrGuardrail {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    SingulrGuardrail::new(built, &environment, base_builder().build().unwrap()).unwrap()
}

pub(crate) fn guardrail(base: &str) -> SingulrGuardrail {
    build(&args_for(base))
}

pub(crate) fn open_guardrail(base: &str) -> SingulrGuardrail {
    let mut built = args_for(base);
    built.block_on_error = Some(false);
    build(&built)
}

#[derive(Clone)]
pub(crate) struct Seen {
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: GuardrailValue,
}

pub(crate) struct MockServer {
    pub(crate) base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    _server: JoinSet<io::Result<()>>,
}

pub(crate) async fn mock(status: StatusCode, reply: &str) -> MockServer {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);
    let reply_for_response = reply.to_owned();
    let handle = move |uri: Uri, headers: HeaderMap, bytes: Bytes| {
        let recorder = Arc::clone(&recorder);
        let answer = reply_for_response.clone();
        async move {
            recorder.lock().unwrap().push(Seen {
                path: uri.path().to_owned(),
                headers: header_text(&headers),
                body: serde_json::from_slice(&bytes).unwrap(),
            });
            (status, [("content-type", "application/json")], answer)
        }
    };
    let (base, server) = serve(Router::new().fallback(handle)).await;
    MockServer {
        base,
        seen,
        _server: server,
    }
}

fn header_text(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|text| (name.to_string(), text.to_owned()))
        })
        .collect()
}

// the listener is closed before the call, so the connect is refused and no server answers
pub(crate) async fn refused_base() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{address}")
}

impl MockServer {
    pub(crate) fn calls(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

async fn serve(router: Router) -> (String, JoinSet<io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server)
}

pub(crate) fn data(json: &str) -> RequestData {
    serde_json::from_str(json).unwrap()
}

pub(crate) fn inputs(json: &str) -> GuardrailApiInputs {
    serde_json::from_str(json).unwrap()
}

pub(crate) fn expects(call: &Seen, expected: &str) {
    assert_eq!(call.body, json_value(expected));
}

pub(crate) fn json_value(text: &str) -> GuardrailValue {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn encoded<T: serde::Serialize>(value: &T) -> GuardrailValue {
    json_value(&serde_json::to_string(value).unwrap())
}
