use std::collections::VecDeque;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::mem;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::sections::xai_oauth_settings::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use rand::rngs::SysError;
use rust_decimal::Decimal;
use serde::Deserialize;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::task::yield_now;
use tokio::time::sleep;
use tracing::field::Field as EventField;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use url::Url;

use crate::xai_login::error::XaiLoginError;
use crate::xai_login::listener::{CallbackServer, CallbackSettings};
use crate::xai_login::pkce::Entropy;
use crate::xai_login::service::{Browser, UserOutput, Uuids, XaiLogin};
use crate::xai_tokens::Clock;
use crate::xai_tokens::XaiOauth;
use crate::xai_tokens::files::LocalFiles;
use crate::xai_tokens::transport::Transport;

pub(crate) const NOW: i64 = 1_000;
pub(crate) const STATE: &str = "state-test";
pub(crate) const NONCE: &str = "nonce-test";
pub(crate) const FRESH: &str = r#"{"access_token":"live-access","refresh_token":"live-refresh","id_token":"live-id","token_type":"Bearer","token_endpoint":"https://auth.x.ai/oauth2/token","expires_at":5000}"#;
pub(crate) const STALE: &str = r#"{"access_token":"old-access","refresh_token":"old-refresh","id_token":"old-id","token_type":"Bearer","token_endpoint":"https://auth.x.ai/oauth2/token","expires_at":1000}"#;
pub(crate) const STALE_WITHOUT_REFRESH: &str = r#"{"access_token":"old-access","token_type":"Bearer","token_endpoint":"https://auth.x.ai/oauth2/token","expires_at":1000}"#;
pub(crate) const TOKEN_REPLY: &str = r#"{"access_token":"new-access","refresh_token":"new-refresh","id_token":"new-id","expires_in":7200}"#;
pub(crate) const DISCOVERY_REPLY: &str = r#"{"authorization_endpoint":"https://auth.x.ai/oauth2/authorize","token_endpoint":"https://auth.x.ai/oauth2/token"}"#;

pub(crate) enum Reply {
    Body(u16, &'static str),
    Fail,
}

#[derive(Debug)]
pub(crate) struct Seen {
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) content_type: String,
    pub(crate) body: String,
}

#[derive(Default)]
pub(crate) struct Script {
    pub(crate) replies: Mutex<VecDeque<Reply>>,
    pub(crate) seen: Mutex<Vec<Seen>>,
}

impl Transport for Script {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        let (content_type, body) = match request.body {
            RequestBody::Bytes {
                content_type: kind,
                data,
            } => (
                kind.to_str().unwrap().to_owned(),
                String::from_utf8(data).unwrap(),
            ),
            RequestBody::Empty => (String::new(), String::new()),
            RequestBody::Json(_) | RequestBody::Multipart { .. } => panic!("unexpected body"),
        };
        self.seen.lock().unwrap().push(Seen {
            method: request.method.to_string(),
            url: request.url.to_string(),
            content_type,
            body,
        });
        yield_now().await;
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("the login sent one request more than the script replies");
        match reply {
            Reply::Body(status, text) => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body: text.as_bytes().to_vec(),
            }),
            Reply::Fail => Err(TransportError::BodyTooLarge { limit: 1 }),
        }
    }
}

pub(crate) struct FixedClock(pub(crate) Decimal);

impl Clock for FixedClock {
    fn now(&self) -> Decimal {
        self.0
    }
}

pub(crate) struct SteppingClock {
    pub(crate) now: Mutex<Decimal>,
    pub(crate) step: Decimal,
}

impl Clock for SteppingClock {
    fn now(&self) -> Decimal {
        let mut now = self.now.lock().unwrap();
        let current = *now;
        *now = current.checked_add(self.step).unwrap();
        current
    }
}

pub(crate) struct TestEntropy {
    pub(crate) bytes: [u8; 32],
    pub(crate) fail: bool,
}

impl Entropy for TestEntropy {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), XaiLoginError> {
        if self.fail {
            return Err(XaiLoginError::Entropy {
                source: SysError::UNEXPECTED,
            });
        }
        bytes.copy_from_slice(&self.bytes);
        Ok(())
    }
}

pub(crate) struct QueueUuids(pub(crate) Mutex<VecDeque<String>>);

impl Uuids for QueueUuids {
    fn next(&self) -> String {
        self.0.lock().unwrap().pop_front().unwrap()
    }
}

pub(crate) struct RecordingBrowser {
    pub(crate) url: Mutex<Option<String>>,
    pub(crate) opens: Mutex<usize>,
    pub(crate) result: bool,
}

impl Browser for RecordingBrowser {
    fn open(&self, url: &str) -> bool {
        {
            let mut opens = self.opens.lock().unwrap();
            *opens = opens.checked_add(1).unwrap();
        };
        *self.url.lock().unwrap() = Some(url.to_owned());
        self.result
    }
}

impl RecordingBrowser {
    pub(crate) fn url(&self) -> Option<String> {
        self.url.lock().unwrap().clone()
    }

    pub(crate) fn opens(&self) -> usize {
        *self.opens.lock().unwrap()
    }
}

pub(crate) struct RecordingOutput {
    pub(crate) text: Mutex<String>,
    pub(crate) fail: bool,
}

impl UserOutput for RecordingOutput {
    fn write(&self, text: &str) -> io::Result<()> {
        if self.fail {
            return Err(io::Error::other("no terminal"));
        }
        {
            let mut buffer = self
                .text
                .lock()
                .map_err(|_error| io::Error::other("the recording lock is poisoned"))?;
            buffer.push_str(text);
        };
        Ok(())
    }
}

impl RecordingOutput {
    pub(crate) fn text(&self) -> String {
        self.text.lock().unwrap().clone()
    }
}

pub(crate) struct BrokenWriter;

impl Write for BrokenWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("closed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) type TestLogin = XaiLogin<
    LocalFiles,
    FixedClock,
    Script,
    TestEntropy,
    QueueUuids,
    RecordingBrowser,
    RecordingOutput,
>;

pub(crate) struct Fixture {
    pub(crate) _dir: TempDir,
    pub(crate) service: TestLogin,
}

impl Fixture {
    pub(crate) fn new(existing: Option<&str>, replies: Vec<Reply>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            token_dir: dir.path().join("xai").to_str().unwrap().to_owned(),
            auth_file: "auth.json".to_owned(),
            api_base: String::new(),
            deployments: Vec::new(),
        };
        let service = login_service(settings, replies);
        let fixture = Self { _dir: dir, service };
        if let Some(text) = existing {
            let path = &fixture.service.oauth.auth_file;
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        fixture
    }

    pub(crate) fn seen(&self) -> Vec<Seen> {
        mem::take(&mut *self.service.oauth.transport.seen.lock().unwrap())
    }

    pub(crate) fn stored_file(&self) -> String {
        fs::read_to_string(&self.service.oauth.auth_file).unwrap()
    }
}

fn login_service(settings: Settings, replies: Vec<Reply>) -> TestLogin {
    XaiLogin {
        oauth: XaiOauth::new(
            settings,
            LocalFiles,
            FixedClock(Decimal::from(NOW)),
            Script {
                replies: Mutex::new(VecDeque::from(replies)),
                seen: Mutex::new(Vec::new()),
            },
            Duration::from_secs(5),
        ),
        entropy: TestEntropy {
            bytes: [7; 32],
            fail: false,
        },
        uuids: QueueUuids(Mutex::new(VecDeque::from([
            STATE.to_owned(),
            NONCE.to_owned(),
        ]))),
        browser: RecordingBrowser {
            url: Mutex::new(None),
            opens: Mutex::new(0),
            result: true,
        },
        output: RecordingOutput {
            text: Mutex::new(String::new()),
            fail: false,
        },
        callbacks: CallbackSettings {
            preferred_port: 0,
            poll: Duration::from_millis(5),
            timeout: Duration::from_secs(30),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) content_type: Option<String>,
    pub(crate) body: String,
}

pub(crate) async fn get(port: u16, target: &str) -> Response {
    let request = format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
    send(port, &request).await
}

pub(crate) async fn send(port: u16, request: &str) -> Response {
    let mut stream = TcpStream::connect(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
        .await
        .unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.unwrap();
    parse_response(&bytes)
}

fn parse_response(bytes: &[u8]) -> Response {
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let content_type =
        lines.find_map(|line| line.strip_prefix("Content-Type: ").map(str::to_owned));
    Response {
        status,
        content_type,
        body: body.to_owned(),
    }
}

pub(crate) fn server_port(server: &CallbackServer) -> u16 {
    Url::parse(&server.redirect_uri).unwrap().port().unwrap()
}

pub(crate) fn port_of(authorize_url: &str) -> u16 {
    let url = Url::parse(authorize_url).unwrap();
    let redirect = url
        .query_pairs()
        .find_map(|(name, value)| (name == "redirect_uri").then(|| value.into_owned()))
        .unwrap();
    Url::parse(&redirect).unwrap().port().unwrap()
}

pub(crate) async fn callback_when<F>(announced: F, target: &str) -> Response
where
    F: Fn() -> Option<String>,
{
    for _attempt in 0_u16..600_u16 {
        if let Some(url) = announced() {
            return get(port_of(&url), target).await;
        }
        sleep(Duration::from_millis(5)).await;
    }
    panic!("the login never announced the authorization URL");
}

#[derive(Deserialize)]
pub(crate) struct Transcript {
    pub(crate) expected_state: String,
    pub(crate) cases: Vec<PinnedCase>,
}

#[derive(Deserialize)]
pub(crate) struct PinnedCase {
    pub(crate) name: String,
    pub(crate) target: String,
    pub(crate) status: u16,
    pub(crate) content_type: Option<String>,
    pub(crate) body: String,
    pub(crate) callback: Option<PinnedCallback>,
}

#[derive(Deserialize)]
pub(crate) struct PinnedCallback {
    pub(crate) code: Option<String>,
    pub(crate) state: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) error_description: Option<String>,
}

pub(crate) fn transcript() -> Transcript {
    serde_json::from_str(include_str!(
        "../../../../../tools/oracle/fixtures/xai-login/callback-transcript.json"
    ))
    .unwrap()
}

#[derive(Clone, Default)]
pub(crate) struct Captured(pub(crate) Arc<Mutex<Vec<String>>>);

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = String::new();
        event.record(&mut |field: &EventField, value: &dyn fmt::Debug| {
            message = format!("{} {value:?}", field.name());
        });
        self.0.lock().unwrap().push(message);
    }
}

pub(crate) fn capture_events() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::registry().with(captured.clone());
    (captured, set_default(subscriber))
}
