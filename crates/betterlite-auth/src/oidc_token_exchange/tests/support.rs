use std::collections::{BTreeMap, VecDeque};
use std::str;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use url::form_urlencoded;

use crate::oidc_token_exchange::decode::ProviderClaims;
use crate::oidc_token_exchange::exchange::TokenExchangeRequest;

pub(crate) const VALID_ID_TOKEN: &str =
    "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ1c2VyLTEiLCJlbWFpbCI6InVAZXhhbXBsZS5jb20ifQ.";

pub(crate) fn token_reply(id_token: Option<&str>) -> String {
    match id_token {
        Some(id_token) => format!(r#"{{"access_token":"access-1","id_token":"{id_token}"}}"#),
        None => r#"{"access_token":"access-1"}"#.to_owned(),
    }
}

pub(crate) fn base_request(provider: &LocalProvider) -> TokenExchangeRequest {
    TokenExchangeRequest {
        authorization_code: "code-1".to_owned(),
        code_verifier: SecretString::from("verifier-1".to_owned()),
        client_id: "client-1".to_owned(),
        client_secret: Some(SecretString::from("client-secret".to_owned())),
        token_endpoint: provider.endpoint("/token"),
        userinfo_endpoint: Some(provider.endpoint("/userinfo")),
        include_client_id: false,
        redirect_url: Some("https://proxy.example/sso/callback".to_owned()),
        additional_headers: Vec::new(),
    }
}

pub(crate) fn client() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::outbound(&policy).unwrap()
}

pub(crate) fn form(body: &str) -> BTreeMap<String, String> {
    form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect()
}

pub(crate) fn text<'claims>(claims: &'claims ProviderClaims, name: &str) -> Option<&'claims str> {
    claims.get(name).and_then(|value| value.json.as_str())
}

pub(crate) struct Reply {
    status: u16,
    body: String,
    headers: Vec<(String, String)>,
}

impl Reply {
    pub(crate) fn json(body: &str) -> Self {
        Self {
            status: 200,
            body: body.to_owned(),
            headers: Vec::new(),
        }
    }

    pub(crate) fn status(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_owned(),
            headers: Vec::new(),
        }
    }

    pub(crate) fn redirect(location: &str) -> Self {
        Self {
            status: 307,
            body: String::new(),
            headers: vec![("location".to_owned(), location.to_owned())],
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Recorded {
    pub(crate) method: String,
    pub(crate) path: String,
    headers: BTreeMap<String, String>,
    pub(crate) body: String,
}

impl Recorded {
    pub(crate) fn header(&self, name: &str) -> &str {
        self.headers.get(name).map_or("", String::as_str)
    }
}

pub(crate) struct LocalProvider {
    url: String,
    requests: Arc<Mutex<Vec<Recorded>>>,
    replies: Arc<Mutex<BTreeMap<String, VecDeque<Reply>>>>,
    _tasks: JoinSet<()>,
}

impl LocalProvider {
    pub(crate) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the test provider binds a loopback port");
        let url = format!(
            "http://{}",
            listener.local_addr().expect("the bound address is known")
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let replies = Arc::new(Mutex::new(BTreeMap::new()));
        let mut tasks = JoinSet::new();
        tasks.spawn(serve(listener, Arc::clone(&requests), Arc::clone(&replies)));
        Self {
            url,
            requests,
            replies,
            _tasks: tasks,
        }
    }

    pub(crate) fn endpoint(&self, path: &str) -> String {
        format!("{}{path}", self.url)
    }

    pub(crate) fn reply(&self, path: &str, reply: Reply) {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(path.to_owned())
            .or_default()
            .push_back(reply);
    }

    pub(crate) fn requests(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

async fn serve(
    listener: TcpListener,
    requests: Arc<Mutex<Vec<Recorded>>>,
    replies: Arc<Mutex<BTreeMap<String, VecDeque<Reply>>>>,
) {
    while let Ok((mut stream, _peer)) = listener.accept().await {
        let Some(recorded) = read_request(&mut stream).await else {
            continue;
        };
        let reply = replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(&recorded.path)
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| Reply::status(599, "the test provider has no reply"));
        requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        write_reply(&mut stream, &reply).await;
    }
}

async fn write_reply(stream: &mut TcpStream, reply: &Reply) {
    let mut text = format!("HTTP/1.1 {} Answer\r\n", reply.status);
    for (name, value) in &reply.headers {
        text.push_str(name);
        text.push_str(": ");
        text.push_str(value);
        text.push_str("\r\n");
    }
    text.push_str("content-length: ");
    text.push_str(&reply.body.len().to_string());
    text.push_str("\r\nconnection: close\r\n\r\n");
    text.push_str(&reply.body);
    drop(stream.write_all(text.as_bytes()).await);
}

async fn read_request(stream: &mut TcpStream) -> Option<Recorded> {
    let mut received = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = stream.read(&mut chunk).await.ok()?;
        received.extend_from_slice(chunk.get(..read)?);
        let Some(head_end) = received.windows(4).position(|window| window == b"\r\n\r\n") else {
            if read == 0 {
                return None;
            }
            continue;
        };
        let head = str::from_utf8(received.get(..head_end)?).ok()?;
        let length = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        let body_start = head_end.checked_add(4)?;
        let body_length = received.len().saturating_sub(body_start);
        if body_length >= length || read == 0 {
            let body = String::from_utf8(received.get(body_start..)?.to_vec()).ok()?;
            return parse_recorded(head, body);
        }
    }
}

fn parse_recorded(head: &str, body: String) -> Option<Recorded> {
    let mut lines = head.lines();
    let mut request = lines.next()?.split_whitespace();
    let method = request.next()?.to_owned();
    let path = request.next()?.to_owned();
    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    Some(Recorded {
        method,
        path,
        headers,
        body,
    })
}
