use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_sdk::proxy_auth::{GenericOAuth2Credential, TokenCredential as _};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use url::{Url, form_urlencoded};

use super::Moment;
use crate::helpers::HelperResult;

#[derive(Debug, Deserialize)]
struct Reply {
    status: u16,
    body: Value,
}

#[derive(Debug, Deserialize)]
struct Call {
    at: i64,
    scope: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Oauth2 {
    client_id: String,
    client_secret: SecretString,
    responses: Vec<Reply>,
    calls: Vec<Call>,
}

struct Endpoint {
    url: Url,
    posts: Arc<Mutex<Vec<String>>>,
    _tasks: JoinSet<io::Result<()>>,
}

async fn read_body(stream: &mut TcpStream) -> io::Result<String> {
    let mut received = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream.read(&mut chunk).await?;
        received.extend_from_slice(chunk.get(..read).unwrap_or_default());
        let text = String::from_utf8_lossy(&received).into_owned();
        if let Some((head, body)) = text.split_once("\r\n\r\n") {
            let wanted = head
                .to_ascii_lowercase()
                .lines()
                .find_map(|line| line.strip_prefix("content-length: ").map(str::to_owned))
                .map_or(Ok(0), |length| length.parse::<usize>())
                .map_err(io::Error::other)?;
            if body.len() >= wanted {
                return Ok(body.to_owned());
            }
        }
        if read == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
    }
}

async fn answer_posts(
    listener: TcpListener,
    replies: Arc<Mutex<VecDeque<(u16, String)>>>,
    posts: Arc<Mutex<Vec<String>>>,
) -> io::Result<()> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let body = read_body(&mut stream).await?;
        posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(body);
        let (status, text) = replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| (500, "{}".to_owned()));
        let length = text.len();
        let answer = format!(
            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {length}\r\nconnection: close\r\n\r\n{text}"
        );
        stream.write_all(answer.as_bytes()).await?;
    }
}

async fn serve(responses: &[Reply]) -> io::Result<Endpoint> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = Url::parse(&format!("http://{}/oauth2/issue", listener.local_addr()?))
        .map_err(io::Error::other)?;
    let replies = responses
        .iter()
        .map(|reply| (reply.status, reply.body.to_string()))
        .collect();
    let posts = Arc::<Mutex<Vec<String>>>::default();
    let mut tasks = JoinSet::new();
    tasks.spawn(answer_posts(
        listener,
        Arc::new(Mutex::new(replies)),
        Arc::clone(&posts),
    ));
    Ok(Endpoint {
        url,
        posts,
        _tasks: tasks,
    })
}

pub(super) async fn run(input: &Oauth2) -> HelperResult {
    let endpoint = serve(&input.responses).await.map_err(super::failure)?;
    let trust = ProxyTrust::default();
    let policy = ProxyPolicy::resolve(&trust, &BTreeMap::new()).map_err(super::failure)?;
    let client = ProxiedClient::build(base_builder(), &Arc::new(policy), TransportLimits::DEFAULT)
        .map_err(super::failure)?;
    let clock = Moment::default();
    let credential = GenericOAuth2Credential {
        client_id: input.client_id.clone(),
        client_secret: input.client_secret.clone(),
        token_url: endpoint.url.clone(),
        client,
        clock: clock.clone(),
        cached_token: Mutex::new(None),
    };
    let mut results = Vec::new();
    for call in &input.calls {
        clock.0.store(call.at, Ordering::SeqCst);
        results.push(match credential.get_token(&call.scope).await {
            Ok(token) => json!({
                "token": token.token.expose_secret(),
                "expires_on": token.expires_on,
            }),
            Err(_) => json!({"error": true}),
        });
    }
    let posts: Vec<BTreeMap<String, String>> = endpoint
        .posts
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|body| {
            form_urlencoded::parse(body.as_bytes())
                .into_owned()
                .collect()
        })
        .collect();
    Ok(json!({"results": results, "posts": posts}))
}
