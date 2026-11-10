use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::io;
use std::iter::repeat_with;
use std::net::IpAddr;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::error::{SsrfError, UserUrlError};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::api::a2a_discover::discovery::{CardResponse, CardSource, CardTransportError};
use crate::auth::auth_identity::AuthContext;

pub(super) fn auth_context(roles: &[Role]) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject: "admin-user".to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: roles.to_vec(),
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey::default(),
        },
    }
}

pub(super) enum ScriptedReply {
    Response { status: u16, body: String },
    Failure,
    Blocked,
}

pub(super) struct ScriptedSource {
    replies: Mutex<VecDeque<ScriptedReply>>,
    urls: Mutex<Vec<String>>,
}

impl ScriptedSource {
    pub(super) fn new(replies: Vec<ScriptedReply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            urls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn failing(count: usize) -> Self {
        Self::new(repeat_with(|| ScriptedReply::Failure).take(count).collect())
    }

    pub(super) fn urls(&self) -> Vec<String> {
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl CardSource for ScriptedSource {
    async fn get(&self, url: &str) -> Result<CardResponse, CardTransportError> {
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(url.to_owned());
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match reply {
            Some(ScriptedReply::Response { status, body }) => Ok(CardResponse {
                status,
                body: body.into_bytes(),
            }),
            Some(ScriptedReply::Blocked) => Err(CardTransportError::Url(UserUrlError::Ssrf(
                SsrfError::BlockedAddress {
                    address: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)),
                },
            ))),
            Some(ScriptedReply::Failure) | None => Err(CardTransportError::Deadline {
                timeout: Duration::from_secs(10),
            }),
        }
    }
}

pub(super) fn proxied_client(read_timeout: Duration) -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let limits = TransportLimits {
        read_timeout,
        ..TransportLimits::DEFAULT
    };
    ProxiedClient::build(base_builder(), &policy, limits).unwrap()
}

pub(super) fn safe_fetcher(allowed_hosts: &[&str]) -> UserUrlFetcher<SystemResolver> {
    let hosts: Vec<String> = allowed_hosts
        .iter()
        .map(|host| (*host).to_owned())
        .collect();
    UserUrlFetcher {
        validator: UrlValidator::new(SystemResolver, &hosts, &Verification::Enabled),
        validation: true,
    }
}

pub(super) enum ReplyKind {
    Json,
    Redirect(String),
    Delayed(Duration),
    Stalled(Duration),
}

pub(super) struct Reply {
    status: u16,
    body: Vec<u8>,
    location: Option<String>,
    delay: Duration,
    stall: Duration,
}

impl Reply {
    fn shaped(kind: ReplyKind, body: &str) -> Self {
        match kind {
            ReplyKind::Json => Self {
                status: 200,
                body: body.as_bytes().to_vec(),
                location: None,
                delay: Duration::ZERO,
                stall: Duration::ZERO,
            },
            ReplyKind::Redirect(target) => Self {
                status: 302,
                body: Vec::new(),
                location: Some(target),
                delay: Duration::ZERO,
                stall: Duration::ZERO,
            },
            ReplyKind::Delayed(delay) => Self {
                status: 200,
                body: body.as_bytes().to_vec(),
                location: None,
                delay,
                stall: Duration::ZERO,
            },
            ReplyKind::Stalled(stall) => Self {
                status: 200,
                body: body.as_bytes().to_vec(),
                location: None,
                delay: Duration::ZERO,
                stall,
            },
        }
    }

    pub(super) fn json(body: &str) -> Self {
        Self::shaped(ReplyKind::Json, body)
    }

    pub(super) fn redirect(target: &str) -> Self {
        Self::shaped(ReplyKind::Redirect(target.to_owned()), "")
    }

    pub(super) fn delayed(delay: Duration, body: &str) -> Self {
        Self::shaped(ReplyKind::Delayed(delay), body)
    }

    pub(super) fn stalled(stall: Duration, body: &str) -> Self {
        Self::shaped(ReplyKind::Stalled(stall), body)
    }
}

pub(super) struct Server {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    tasks: JoinSet<io::Result<()>>,
}

impl Server {
    pub(super) async fn start(table: Vec<(&str, Reply)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let routes: Arc<BTreeMap<String, Reply>> = Arc::new(
            table
                .into_iter()
                .map(|(path, reply)| (path.to_owned(), reply))
                .collect(),
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut tasks = JoinSet::new();
        let accept_requests = Arc::clone(&requests);
        tasks.spawn(async move {
            let mut connections = JoinSet::new();
            while let Ok((stream, _)) = listener.accept().await {
                connections.spawn(converse(
                    stream,
                    Arc::clone(&routes),
                    Arc::clone(&accept_requests),
                ));
            }
            Ok(())
        });
        Self {
            address,
            requests,
            tasks,
        }
    }

    pub(super) fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    pub(super) fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

async fn converse(
    mut stream: TcpStream,
    routes: Arc<BTreeMap<String, Reply>>,
    requests: Arc<Mutex<Vec<String>>>,
) -> io::Result<()> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
        let count = stream.read(&mut chunk).await?;
        if count == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..count]);
    }
    let head = String::from_utf8(buffer).unwrap_or_default();
    let target = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(target.to_owned());
    let path = target.split('?').next().unwrap_or("/");
    let Some(reply) = routes.get(path) else {
        return write_reply(&mut stream, 404, None, &[], Duration::ZERO).await;
    };
    if !reply.delay.is_zero() {
        sleep(reply.delay).await;
    }
    write_reply(
        &mut stream,
        reply.status,
        reply.location.as_deref(),
        &reply.body,
        reply.stall,
    )
    .await
}

async fn write_reply(
    stream: &mut TcpStream,
    status: u16,
    location: Option<&str>,
    body: &[u8],
    stall: Duration,
) -> io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(target) = location {
        head.push_str("Location: ");
        head.push_str(target);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    if !stall.is_zero() {
        sleep(stall).await;
    }
    stream.write_all(body).await
}
