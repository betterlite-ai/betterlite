use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io::{Error as IoError, Result as IoResult, Write};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt;

use super::super::init::ZscalerArgs;
use super::super::{Inputs, ZscalerAiGuard};

pub(super) struct Seen {
    pub(super) method: Method,
    pub(super) path: String,
    pub(super) headers: HeaderMap,
    pub(super) body: GuardrailValue,
}

pub(super) struct Mock {
    pub(super) base: String,
    pub(super) seen: Arc<Mutex<Vec<Seen>>>,
    _server: JoinSet<IoResult<()>>,
}

pub(super) async fn mock(status: u16, reply: &str) -> Mock {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&recorded);
    let answer_text = reply.to_owned();
    let answer = move |method: Method, uri: Uri, headers: HeaderMap, bytes: Bytes| {
        let captures = Arc::clone(&sink);
        let answer_body = answer_text.clone();
        async move {
            let body =
                serde_json::from_slice::<GuardrailValue>(&bytes).unwrap_or(GuardrailValue::Null);
            captures.lock().unwrap().push(Seen {
                method,
                path: uri.path().to_owned(),
                headers,
                body,
            });
            (
                StatusCode::from_u16(status).unwrap(),
                [("content-type", "application/json")],
                answer_body,
            )
        }
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, Router::new().fallback(answer)).into_future());
    Mock {
        base,
        seen: recorded,
        _server: server,
    }
}

pub(super) fn args() -> ZscalerArgs {
    ZscalerArgs {
        name: "zscaler-ai-guard".to_owned(),
        api_base: None,
        api_key: Some(SecretString::from("oracle-key")),
        policy_id: Some(42),
        send_user_api_key_alias: Some(false),
        send_user_api_key_user_id: Some(false),
        send_user_api_key_team_id: Some(false),
        timeout: Some(2.0),
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: false,
    }
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn guardrail(base: &str) -> ZscalerAiGuard {
    let mut args = args();
    args.api_base = Some(base.to_owned());
    let environment: BTreeMap<String, String> = BTreeMap::new();
    ZscalerAiGuard::new(args, &environment, base_builder().build().unwrap()).unwrap()
}

pub(super) fn inputs(texts: &[&str]) -> Inputs {
    Inputs {
        texts: texts.iter().map(|text| (*text).to_owned()).collect(),
        rest: BTreeMap::new(),
    }
}

pub(super) fn request_data(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).unwrap()
}

pub(super) fn body(json: &str) -> GuardrailValue {
    serde_json::from_str(json).unwrap()
}

pub(super) fn header(seen: &Seen, name: &str) -> String {
    seen.headers.get(name).unwrap().to_str().unwrap().to_owned()
}

pub(super) fn captured<T, F>(run: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&recorded);
    let subscriber = fmt()
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .with_writer(move || Captured(Arc::clone(&sink)))
        .finish();
    let result = with_default(subscriber, run);
    let bytes = recorded.lock().unwrap().clone();
    (result, String::from_utf8(bytes).unwrap())
}

struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| IoError::other("the captured log lock is poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}
