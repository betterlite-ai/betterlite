use std::collections::BTreeMap;
use std::net::TcpListener as StdTcpListener;
use std::str;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_spec::{PromptLiteLLMParams, PromptSpec};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::bitbucket_client::config::BitBucketConfig;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

pub(crate) const CLOSED_BASE: &str = "http://127.0.0.1:1/2.0";

pub(crate) fn config(base_url: &str) -> BitBucketConfig {
    BitBucketConfig {
        workspace: Some("ws".to_owned()),
        repository: Some("repo".to_owned()),
        access_token: Some(SecretString::from("token")),
        branch: None,
        base_url: Some(base_url.to_owned()),
        auth_method: None,
        username: None,
        unrecognized: BTreeMap::new(),
    }
}

pub(crate) fn transport() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("proxy policy"),
    );
    ProxiedClient::outbound(&policy).expect("transport")
}

pub(crate) fn params(prompt_id: Option<&str>, base_url: &str) -> PromptLiteLLMParams {
    let prompt_id_json = prompt_id.map_or_else(|| "null".to_owned(), |id| format!("{id:?}"));
    let text = format!(
        r#"{{"prompt_integration": "bitbucket", "prompt_id": {prompt_id_json}, "bitbucket_config": {{"workspace": "ws", "repository": "repo", "access_token": "token", "": {base_url:?}}}}}"#
    );
    serde_json::from_str(&text).expect("litellm params")
}

pub(crate) fn params_without_config() -> PromptLiteLLMParams {
    serde_json::from_str(r#"{"prompt_integration": "bitbucket", "prompt_id": "greeting"}"#)
        .expect("litellm params")
}

pub(crate) fn prompt_spec(prompt_id: &str) -> PromptSpec {
    let text = format!(
        r#"{{"prompt_id": {prompt_id:?}, "litellm_params": {{"prompt_integration": "bitbucket"}}}}"#
    );
    serde_json::from_str(&text).expect("prompt spec")
}

pub(crate) fn messages(pairs: &[(&str, &str)]) -> Vec<ContractJson> {
    pairs
        .iter()
        .map(|(role, content)| {
            let mut message = IndexMap::new();
            message.insert("role".to_owned(), ContractJson::Text((*role).to_owned()));
            message.insert(
                "content".to_owned(),
                ContractJson::Text((*content).to_owned()),
            );
            ContractJson::Object(message)
        })
        .collect()
}

pub(crate) fn number(object: &Object, key: &str) -> Option<f64> {
    let value = object.get(key)?;
    match value {
        Json::Number(number) => number.as_f64(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub(crate) struct Server {
    pub(crate) base_url: String,
    pub(crate) requests: Arc<AtomicUsize>,
    _tasks: JoinSet<()>,
}

impl Server {
    pub(crate) fn start(served: &[(&str, u16, &str)]) -> Self {
        let std_listener = StdTcpListener::bind("127.0.0.1:0").expect("bind");
        std_listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let address = std_listener.local_addr().expect("listener address");
        let routes: BTreeMap<String, (u16, String)> = served
            .iter()
            .map(|(name, status, body)| ((*name).to_owned(), (*status, (*body).to_owned())))
            .collect();
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&requests);
        let mut tasks = JoinSet::new();
        tasks.spawn(serve(std_listener, routes, counter));
        Self {
            base_url: format!("http://{address}/2.0"),
            requests,
            _tasks: tasks,
        }
    }

    pub(crate) fn request_count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

async fn serve(
    std_listener: StdTcpListener,
    routes: BTreeMap<String, (u16, String)>,
    counter: Arc<AtomicUsize>,
) {
    let listener = TcpListener::from_std(std_listener).expect("async listener");
    while let Ok((mut stream, _)) = listener.accept().await {
        counter.fetch_add(1, Ordering::SeqCst);
        answer(&mut stream, &routes).await;
    }
}

async fn answer(stream: &mut TcpStream, routes: &BTreeMap<String, (u16, String)>) {
    let mut head = Vec::new();
    loop {
        let mut chunk = [0_u8; 512];
        let Ok(read) = stream.read(&mut chunk).await else {
            return;
        };
        if read == 0 {
            return;
        }
        let Some(part) = chunk.get(..read) else {
            return;
        };
        head.extend_from_slice(part);
        if head.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let Ok(request) = str::from_utf8(&head) else {
        return;
    };
    let Some(path) = request.split_whitespace().nth(1) else {
        return;
    };
    let file = path.rsplit('/').next().unwrap_or_default();
    let (status, body) = match routes.get(file) {
        Some((status, body)) => (*status, body.as_str()),
        None => (404, ""),
    };
    let reply = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    drop(stream.write_all(reply.as_bytes()).await);
}
