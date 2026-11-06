use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::transport_error::TransportError;
use secrecy::SecretString;
use tokio::io::AsyncWriteExt as _;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::sleep;

use super::support::{FakeClock, PROMPT_BODY, fake_clock};
use crate::humanloop::http_transport::{
    HttpPromptTransport, HttpTransportError, MAX_PROMPT_BODY_BYTES, PROMPT_CONNECT_TIMEOUT,
    PROMPT_DEADLINE,
};
use crate::humanloop::prompt_cache::PromptCache;
use crate::humanloop::prompt_manager::{FetchError, HumanLoopPromptManager};

const OVERSIZED_BODY_BYTES: usize = 2_000_000;

enum Scenario {
    Stall,
    BodyTooLarge,
    Redirect { redirected: Arc<AtomicBool> },
}

impl Clone for Scenario {
    fn clone(&self) -> Self {
        match self {
            Self::Stall => Self::Stall,
            Self::BodyTooLarge => Self::BodyTooLarge,
            Self::Redirect { redirected } => Self::Redirect {
                redirected: Arc::clone(redirected),
            },
        }
    }
}

struct MockServer {
    prompt_url: String,
    tasks: JoinSet<()>,
}

impl MockServer {
    async fn start(scenario: Scenario) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let prompt_url = format!("http://{address}/v5/prompts/");
        let redirect_target = prompt_url.clone();
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            let mut connections = JoinSet::new();
            while let Ok((stream, _peer)) = listener.accept().await {
                connections.spawn(serve_connection(
                    stream,
                    scenario.clone(),
                    redirect_target.clone(),
                ));
            }
        });
        Self { prompt_url, tasks }
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

async fn serve_connection(mut stream: TcpStream, scenario: Scenario, redirect_target: String) {
    let response = match scenario {
        Scenario::Stall => {
            sleep(Duration::from_secs(2)).await;
            return;
        }
        Scenario::BodyTooLarge => format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {OVERSIZED_BODY_BYTES}\r\nConnection: close\r\n\r\n"
        ),
        Scenario::Redirect { redirected } => {
            if redirected.swap(true, Ordering::SeqCst) {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{PROMPT_BODY}",
                    PROMPT_BODY.len()
                )
            } else {
                format!(
                    "HTTP/1.1 302 Found\r\nLocation: {redirect_target}pr_target\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
            }
        }
    };
    let _written = stream.write_all(response.as_bytes()).await;
    let _shutdown = stream.shutdown().await;
}

fn http_transport(deadline: Duration) -> HttpPromptTransport {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("proxy policy"),
    );
    let mut transport = HttpPromptTransport::new(&policy).expect("transport");
    transport.deadline = deadline;
    transport
}

fn manager_at(
    transport: HttpPromptTransport,
    prompt_url: &str,
) -> HumanLoopPromptManager<HttpPromptTransport, FakeClock> {
    HumanLoopPromptManager {
        transport,
        cache: Mutex::new(PromptCache {
            clock: fake_clock(),
            ttl: Duration::from_secs(60),
            entries: HashMap::new(),
        }),
        prompt_url: prompt_url.to_owned(),
    }
}

#[tokio::test]
async fn the_http_transport_uses_litellm_timeouts_and_caps() {
    let transport = http_transport(PROMPT_DEADLINE);
    assert_eq!(transport.deadline, PROMPT_DEADLINE);
    assert_eq!(
        transport.client.limits.connect_timeout,
        PROMPT_CONNECT_TIMEOUT
    );
    assert_eq!(transport.client.limits.body_bytes, MAX_PROMPT_BODY_BYTES);
}

#[tokio::test]
async fn a_stalled_response_ends_in_a_transport_error_within_the_deadline() {
    let server = MockServer::start(Scenario::Stall).await;
    let prompts = manager_at(
        http_transport(Duration::from_millis(200)),
        &server.prompt_url,
    );
    let started = Instant::now();
    let error = prompts
        .resolve("pr_abc123", &SecretString::from("hl-key"), |prompt| {
            prompt.clone()
        })
        .await
        .expect_err("stalled");
    assert!(matches!(error, FetchError::Transport { .. }));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn a_response_body_over_the_cap_is_rejected() {
    let server = MockServer::start(Scenario::BodyTooLarge).await;
    let prompts = manager_at(http_transport(Duration::from_secs(5)), &server.prompt_url);
    let error = prompts
        .resolve("pr_abc123", &SecretString::from("hl-key"), |prompt| {
            prompt.clone()
        })
        .await
        .expect_err("oversized");
    let FetchError::Transport { source } = error else {
        panic!("expected a transport error");
    };
    assert!(matches!(
        source,
        HttpTransportError::Transport(TransportError::BodyTooLarge { limit })
            if limit == MAX_PROMPT_BODY_BYTES
    ));
}

#[tokio::test]
async fn a_redirect_is_followed_like_httpx() {
    let server = MockServer::start(Scenario::Redirect {
        redirected: Arc::new(AtomicBool::new(false)),
    })
    .await;
    let prompts = manager_at(http_transport(Duration::from_secs(5)), &server.prompt_url);
    let prompt = prompts
        .resolve("pr_abc123", &SecretString::from("hl-key"), |prompt| {
            prompt.clone()
        })
        .await
        .expect("prompt");
    assert_eq!(prompt.prompt_template_model.as_deref(), Some("gpt-4o"));
}
