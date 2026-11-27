use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io;
use std::str::from_utf8;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::oauth_tokens::Settings;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tracing::field::{Field, Visit};
use tracing::subscriber::{self, DefaultGuard};
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::{Layer, Registry};

use crate::oauth_tokens::transport::{
    TokenInfoRequest, TokenInfoResponse, TokenInfoTransport, TokenInfoTransportError,
};
use crate::trusted_network::cidr::CidrRanges;
use crate::trusted_network::request::NetworkRequest;
use crate::trusted_network::settings::TrustedNetworkSettings;

pub(super) const ENDPOINT: &str = "https://idp.example/introspect";
pub(super) const TOKEN: &str = "opaque-token-1";
pub(super) const CLIENT_SECRET: &str = "client-secret-value";

pub(super) fn settings(
    endpoint: Option<&str>,
    client_id: Option<&str>,
    client_secret: Option<&str>,
) -> Settings {
    Settings {
        token_info_endpoint: endpoint.map(str::to_owned),
        user_id_field_name: "sub".to_owned(),
        user_role_field_name: "role".to_owned(),
        user_team_id_field_name: "team_id".to_owned(),
        client_id: client_id.map(str::to_owned),
        client_secret: client_secret.map(|value| SecretString::from(value.to_owned())),
        config_mappings: None,
        enable_oauth2_auth: true,
        enable_oauth2_proxy_auth: false,
    }
}

pub(super) fn token() -> SecretString {
    SecretString::from(TOKEN.to_owned())
}

pub(super) struct Scripted {
    status: u16,
    body: String,
    failure: Mutex<Option<TransportError>>,
    recorded: Mutex<Vec<TokenInfoRequest>>,
}

impl Scripted {
    pub(super) fn answering(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_owned(),
            failure: Mutex::new(None),
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn failing(source: TransportError) -> Self {
        Self {
            status: 0,
            body: String::new(),
            failure: Mutex::new(Some(source)),
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn take(&self) -> TokenInfoRequest {
        let mut requests = self.recorded.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(requests.len(), 1, "the check sends exactly one request");
        requests.pop().expect("the request was recorded")
    }

    pub(super) fn sent(&self) -> usize {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

#[async_trait]
impl TokenInfoTransport for Scripted {
    async fn send(
        &self,
        request: TokenInfoRequest,
    ) -> Result<TokenInfoResponse, TokenInfoTransportError> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        if let Some(source) = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            return Err(TokenInfoTransportError::Failed { source });
        }
        Ok(TokenInfoResponse {
            status: self.status,
            body: self.body.as_bytes().to_vec(),
        })
    }
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (raw_name, raw_value) in pairs {
        let name = HeaderName::from_bytes(raw_name.as_bytes()).expect("the header name parses");
        let value = HeaderValue::from_str(raw_value).expect("the header value parses");
        headers.insert(name, value);
    }
    headers
}

pub(super) fn mappings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(field, header)| ((*field).to_owned(), (*header).to_owned()))
        .collect()
}

pub(super) fn network<'a>(
    peer: Option<&'a str>,
    forwarded_for: Option<&'a str>,
) -> NetworkRequest<'a> {
    NetworkRequest {
        peer,
        x_forwarded_for: forwarded_for,
        x_forwarded_proto: None,
        host: Some("proxy.example"),
        scheme: "https",
    }
}

pub(super) fn trusted(range: &str) -> TrustedNetworkSettings {
    TrustedNetworkSettings {
        proxy_base_url: String::new(),
        use_x_forwarded_for: false,
        trusted_proxy_ranges: Some(CidrRanges::List(vec![range.to_owned()])),
        mcp_trusted_proxy_ranges: None,
        mcp_xff_num_trusted_hops: None,
    }
}

pub(super) fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .expect("the proxy policy resolves");
    ProxiedClient::outbound(&Arc::new(policy)).expect("the outbound client builds")
}

pub(super) struct Seen {
    pub(super) method: String,
    pub(super) target: String,
    pub(super) content_type: Option<String>,
    pub(super) authorization: Option<String>,
    pub(super) body: String,
}

pub(super) struct Idp {
    pub(super) base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    tasks: JoinSet<()>,
}

impl Idp {
    pub(super) fn take(&self) -> Vec<Seen> {
        std::mem::take(&mut *self.seen.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl Drop for Idp {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

enum Replies {
    Answering(u16, String),
    Redirecting(u16, String),
}

// a loopback identity provider the production ProxiedClient transport reaches over TCP
pub(super) async fn serve_idp(status: u16, body: &str) -> Idp {
    let replies = Replies::Answering(status, body.to_owned());
    serve_idp_with(replies).await
}

// the first request is answered with a 302 to /token on the same origin, and the next one with the document
pub(super) async fn serve_redirecting_idp(status: u16, body: &str) -> Idp {
    let replies = Replies::Redirecting(status, body.to_owned());
    serve_idp_with(replies).await
}

async fn serve_idp_with(replies: Replies) -> Idp {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the identity provider binds");
    let address = listener
        .local_addr()
        .expect("the identity provider address is known");
    let seen: Arc<Mutex<Vec<Seen>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&seen);
    let mut tasks = JoinSet::new();
    tasks.spawn(accept(listener, replies, recorded));
    Idp {
        base: format!("http://{address}"),
        seen,
        tasks,
    }
}

async fn accept(listener: TcpListener, replies: Replies, seen: Arc<Mutex<Vec<Seen>>>) {
    let mut first = true;
    loop {
        let Ok((mut stream, _peer)) = listener.accept().await else {
            return;
        };
        let Ok(request) = read_request(&mut stream).await else {
            continue;
        };
        seen.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let response = reply(&listener, &replies, first);
        first = false;
        if stream.write_all(response.as_bytes()).await.is_ok() {
            drop(stream.flush().await);
        }
    }
}

fn reply(listener: &TcpListener, replies: &Replies, first: bool) -> String {
    match replies {
        Replies::Answering(status, body) => answer(*status, body),
        Replies::Redirecting(status, body) => {
            if first {
                redirect(listener)
            } else {
                answer(*status, body)
            }
        }
    }
}

fn answer(status: u16, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn redirect(listener: &TcpListener) -> String {
    let address = listener
        .local_addr()
        .expect("the identity provider address is known");
    format!(
        "HTTP/1.1 302 Found\r\nlocation: http://{address}/token\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
    )
}

struct Head {
    method: String,
    target: String,
    content_type: Option<String>,
    authorization: Option<String>,
    length: usize,
}

async fn read_request(stream: &mut TcpStream) -> io::Result<Seen> {
    let mut buffer = Vec::new();
    let head_end = read_head(stream, &mut buffer).await?;
    let head = parse_head(&buffer[..head_end])?;
    let body = read_body(stream, &mut buffer, head_end, head.length).await?;
    Ok(Seen {
        method: head.method,
        target: head.target,
        content_type: head.content_type,
        authorization: head.authorization,
        body,
    })
}

async fn read_head(stream: &mut TcpStream, buffer: &mut Vec<u8>) -> io::Result<usize> {
    loop {
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            return Ok(position.saturating_add(4));
        }
        if stream.read(buffer).await? == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
    }
}

fn parse_head(head: &[u8]) -> io::Result<Head> {
    let text = from_utf8(head).map_err(|_error| io::Error::from(io::ErrorKind::InvalidData))?;
    let mut lines = text.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split(' ');
    let method = request_line.next().unwrap_or_default().to_owned();
    let target = request_line.next().unwrap_or_default().to_owned();
    let mut length = 0_usize;
    let mut content_type = None;
    let mut authorization = None;
    for line in lines {
        let Some((name, raw_value)) = line.split_once(':') else {
            continue;
        };
        let value = raw_value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = value.parse().unwrap_or_default(),
            "content-type" => content_type = Some(value.to_owned()),
            "authorization" => authorization = Some(value.to_owned()),
            _ => {}
        }
    }
    Ok(Head {
        method,
        target,
        content_type,
        authorization,
        length,
    })
}

async fn read_body(
    stream: &mut TcpStream,
    buffer: &mut Vec<u8>,
    head_end: usize,
    length: usize,
) -> io::Result<String> {
    let end = head_end.saturating_add(length);
    while buffer.len() < end {
        if stream.read(buffer).await? == 0 {
            break;
        }
    }
    Ok(from_utf8(&buffer[head_end..end.min(buffer.len())])
        .unwrap_or_default()
        .to_owned())
}

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<String>>>);

struct RecordedLog(String);

impl Visit for RecordedLog {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_owned();
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: tracing::Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, Inner>) {
        let mut recorded = RecordedLog(String::new());
        event.record(&mut recorded);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{} {}", event.metadata().level(), recorded.0));
    }
}

pub(super) fn capture(logs: &Logs) -> DefaultGuard {
    subscriber::set_default(Registry::default().with(logs.clone()))
}

pub(super) fn events(logs: &Logs) -> Vec<String> {
    let recorded = logs.0.lock().unwrap_or_else(PoisonError::into_inner);
    recorded.clone()
}
