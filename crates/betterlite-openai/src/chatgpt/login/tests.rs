#![cfg(test)]

use std::collections::VecDeque;
use std::fs;
use std::future::{pending, ready};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use betterlite_config::sections::chatgpt_token_store::Settings;
use betterlite_http::proxy::transport_error::TransportError;
use futures_util::FutureExt as _;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
use tracing::subscriber::set_default;

use super::codec::{
    AuthorizationReply, DecodeError, DeviceReply, REPLY_LIMIT, TokenReply, decode_chatgpt_oauth,
};
use super::error::{Failure, LoginError};
use super::http::{AuthPost, AuthResponse, AuthTransport, Body, HttpFailure};
use super::{
    ChatGptLogin, DEVICE_CODE_URL, DEVICE_TOKEN_URL, DeviceDisplay, OAUTH_TOKEN_URL, Pause,
};
use crate::chatgpt::token_store::codec::{AuthRecord, decode_auth_record};
use crate::chatgpt::token_store::files::{FileAccess, StoreEvent};
use crate::chatgpt::token_store::member::Member;
use crate::chatgpt::token_store::{ChatGptTokenStore, Clock};

const DEVICE: &str = r#"{"device_auth_id":"device-secret","user_code":"ABCD-1234","interval":"7"}"#;
const PENDING: &str = r#"{"authorization_code":"","code_challenge":"c","code_verifier":"v"}"#;
const AUTHORIZED: &str =
    r#"{"authorization_code":"auth&code","code_challenge":"c","code_verifier":"verifier"}"#;

type Shared = Arc<Mutex<Option<Vec<u8>>>>;

fn jwt(exp: u64) -> String {
    let claims =
        format!(r#"{{"exp":{exp},"https://api.openai.com/auth":{{"chatgpt_account_id":"acct"}}}}"#);
    format!("h.{}.s", URL_SAFE_NO_PAD.encode(claims))
}

fn tokens(access: &str, refresh: Option<&str>, id: &str) -> String {
    let suffix = refresh.map_or(String::new(), |value| {
        format!(r#","refresh_token":"{value}""#)
    });
    format!(r#"{{"access_token":"{access}","id_token":"{id}"{suffix}}}"#)
}

fn record(access: &str, refresh: &str, expires: u64) -> Vec<u8> {
    format!(
        r#"{{"access_token":"{access}","refresh_token":"{refresh}","id_token":"old-id","expires_at":{expires}}}"#
    )
    .into_bytes()
}

struct Files {
    content: Shared,
    writes: Vec<Vec<u8>>,
    events: Vec<StoreEvent>,
}

impl FileAccess for Files {
    fn ensure_dir(&mut self, _path: &Path) -> io::Result<()> {
        Ok(())
    }
    fn read(&mut self, _path: &Path) -> io::Result<Vec<u8>> {
        let content = self.content.lock().unwrap_or_else(PoisonError::into_inner);
        content
            .clone()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
    fn write(&mut self, _path: &Path, record: &AuthRecord) -> io::Result<()> {
        let bytes = serde_json::to_vec(record)?;
        *self.content.lock().unwrap_or_else(PoisonError::into_inner) = Some(bytes.clone());
        self.writes.push(bytes);
        Ok(())
    }
    fn report(&mut self, event: StoreEvent) {
        self.events.push(event);
    }
}

struct Time {
    now: Decimal,
    sleeps: Vec<Duration>,
    pauses: Vec<Duration>,
    arrival: Option<(Shared, Vec<u8>)>,
    hang: bool,
}

impl Time {
    fn advance(&mut self, duration: Duration) {
        let seconds = Decimal::from(duration.as_secs());
        self.now = self.now.checked_add(seconds).unwrap();
    }
}

impl Clock for Time {
    fn now(&self) -> Decimal {
        self.now
    }
    fn sleep(&mut self, duration: Duration) {
        self.sleeps.push(duration);
        self.advance(duration);
        if let Some((shared, bytes)) = self.arrival.take() {
            *shared.lock().unwrap() = Some(bytes);
        }
    }
}

impl Pause for Time {
    async fn pause(&mut self, duration: Duration) {
        if self.hang {
            pending::<()>().await;
        }
        self.pauses.push(duration);
        self.advance(duration);
    }
}

struct Posted {
    url: &'static str,
    json: bool,
    body: String,
}

#[derive(Clone)]
enum Reply {
    Answer(u16, String),
    Unreachable,
}

#[derive(Default)]
struct Script {
    replies: VecDeque<Reply>,
    repeat: Option<Reply>,
    posts: Vec<Posted>,
}

impl AuthTransport for Script {
    fn post(
        &mut self,
        request: AuthPost,
    ) -> impl Future<Output = Result<AuthResponse, HttpFailure>> + Send {
        let (json, text) = match request.body {
            Body::Json(text) => (true, text),
            Body::Form(text) => (false, text),
        };
        self.posts.push(Posted {
            url: request.url,
            json,
            body: text.expose_secret().to_owned(),
        });
        let reply = self
            .replies
            .pop_front()
            .or_else(|| self.repeat.clone())
            .unwrap_or(Reply::Unreachable);
        ready(match reply {
            Reply::Answer(status, body) => Ok(AuthResponse {
                status,
                body: body.into_bytes(),
            }),
            Reply::Unreachable => Err(HttpFailure::Transport(TransportError::Closed)),
        })
    }
}

#[derive(Default)]
struct Shown {
    prompts: Vec<(String, String)>,
}

impl DeviceDisplay for Shown {
    fn show(&mut self, verify_url: &str, user_code: &SecretString) {
        self.prompts
            .push((verify_url.to_owned(), user_code.expose_secret().to_owned()));
    }
}

type Subject = ChatGptLogin<Files, Time, Script, Shown>;

fn reply(status: u16, body: &str) -> Reply {
    Reply::Answer(status, body.to_owned())
}

fn subject(content: Option<Vec<u8>>, now: i64, replies: Vec<Reply>) -> Subject {
    let shared: Shared = Arc::new(Mutex::new(content));
    let settings = Settings {
        token_dir: "tokens".to_owned(),
        auth_file: "auth.json".to_owned(),
        api_base: "https://configured".to_owned(),
    };
    let files = Files {
        content: Arc::clone(&shared),
        writes: Vec::new(),
        events: Vec::new(),
    };
    let clock = Time {
        now: Decimal::from(now),
        sleeps: Vec::new(),
        pauses: Vec::new(),
        arrival: None,
        hang: false,
    };
    ChatGptLogin {
        store: ChatGptTokenStore::new(settings, files, clock).unwrap(),
        http: Script {
            replies: VecDeque::from(replies),
            repeat: None,
            posts: Vec::new(),
        },
        display: Shown::default(),
    }
}

fn stored(subject: &Subject) -> AuthRecord {
    let bytes = subject.store.files.content.lock().unwrap().clone().unwrap();
    decode_auth_record(&bytes).unwrap()
}

fn text(member: Option<&Member<SecretString>>) -> &str {
    member.and_then(Member::known).unwrap().expose_secret()
}

#[tokio::test]
async fn cached_token_makes_no_request() {
    let access = jwt(4_000_000_000);
    let mut subject = subject(
        Some(record(&access, "refresh", 4_000_000_000)),
        1000,
        vec![],
    );
    let token = subject.get_access_token().await.unwrap();
    assert_eq!(token.expose_secret(), access);
    assert!(subject.http.posts.is_empty());
    assert!(subject.store.files.writes.is_empty());
}

#[rstest]
#[case::keeps_new(Some("rotated"), "rotated")]
#[case::retains_previous(None, "old-refresh")]
#[case::empty_retains_previous(Some(""), "old-refresh")]
#[tokio::test]
async fn refresh_posts_json_and_rebuilds_record(
    #[case] returned: Option<&str>,
    #[case] kept: &str,
) {
    let fresh = jwt(4_000_000_000);
    let body = tokens(&fresh, returned, &fresh);
    let mut subject = subject(
        Some(record("stale", "old-refresh", 10)),
        1000,
        vec![reply(200, &body)],
    );
    let token = subject.get_access_token().await.unwrap();
    assert_eq!(token.expose_secret(), fresh);
    let post = &subject.http.posts[0];
    assert_eq!(post.url, OAUTH_TOKEN_URL);
    assert!(post.json);
    assert_eq!(
        post.body,
        r#"{"client_id":"app_EMoamEEZ73f0CkXaXp7hrann","grant_type":"refresh_token","refresh_token":"old-refresh","scope":"openid profile email"}"#
    );
    let written = stored(&subject);
    assert_eq!(text(written.refresh_token.as_ref()), kept);
    assert_eq!(text(written.access_token.as_ref()), fresh);
    assert_eq!(
        written.account_id.as_ref().and_then(Member::known),
        Some(&"acct".to_owned())
    );
    assert_eq!(
        written
            .expires_at
            .as_ref()
            .and_then(Member::known)
            .and_then(Number::as_i64),
        Some(4_000_000_000)
    );
}

#[rstest]
#[case::refused(vec![reply(401, "{}")], 401)]
#[case::unreachable(vec![], 400)]
#[case::not_json(vec![reply(200, "nope")], 400)]
#[case::missing_id_token(vec![reply(200, r#"{"access_token":"a"}"#)], 400)]
#[tokio::test]
async fn refresh_failures_keep_class_and_status(#[case] replies: Vec<Reply>, #[case] status: u16) {
    let mut subject = subject(None, 1000, replies);
    let error = subject
        .refresh_tokens(&SecretString::from("refresh"))
        .await
        .unwrap_err();
    assert!(matches!(error, LoginError::Refresh(_)));
    assert_eq!(error.status(), status);
    assert!(subject.store.files.writes.is_empty());
}

#[tokio::test]
async fn failed_refresh_relogs_in_with_ordered_requests() {
    let fresh = jwt(4_000_000_000);
    let replies = vec![
        reply(401, "{}"),
        reply(200, DEVICE),
        reply(403, ""),
        reply(404, ""),
        reply(200, PENDING),
        reply(200, AUTHORIZED),
        reply(200, &tokens(&fresh, Some("new-refresh"), &fresh)),
    ];
    let mut subject = subject(Some(record("stale", "old-refresh", 10)), 1000, replies);
    let token = subject.get_access_token().await.unwrap();
    assert_eq!(token.expose_secret(), fresh);
    let urls: Vec<_> = subject.http.posts.iter().map(|post| post.url).collect();
    assert_eq!(
        urls,
        [
            OAUTH_TOKEN_URL,
            DEVICE_CODE_URL,
            DEVICE_TOKEN_URL,
            DEVICE_TOKEN_URL,
            DEVICE_TOKEN_URL,
            DEVICE_TOKEN_URL,
            OAUTH_TOKEN_URL
        ]
    );
    assert_eq!(
        subject.http.posts[1].body,
        r#"{"client_id":"app_EMoamEEZ73f0CkXaXp7hrann"}"#
    );
    assert_eq!(
        subject.http.posts[2].body,
        r#"{"device_auth_id":"device-secret","user_code":"ABCD-1234"}"#
    );
    assert_eq!(subject.store.clock.pauses, [Duration::from_secs(7); 3]);
    let written = stored(&subject);
    assert_eq!(text(written.refresh_token.as_ref()), "new-refresh");
    assert!(written.device_code_requested_at.is_none());
    assert_eq!(subject.store.files.writes.len(), 2);
}

#[tokio::test]
async fn device_code_is_shown_privately_and_recorded_after_the_request() {
    let fresh = jwt(4_000_000_000);
    let replies = vec![
        reply(200, r#"{"device_auth_id":"id","usercode":"WXYZ"}"#),
        reply(200, AUTHORIZED),
        reply(200, &tokens(&fresh, Some("r"), &fresh)),
    ];
    let mut subject = subject(None, 1234, replies);
    subject.get_access_token().await.unwrap();
    assert_eq!(
        subject.display.prompts,
        [(
            "https://auth.openai.com/codex/device".to_owned(),
            "WXYZ".to_owned()
        )]
    );
    let first = decode_auth_record(&subject.store.files.writes[0]).unwrap();
    assert_eq!(
        first
            .device_code_requested_at
            .as_ref()
            .and_then(Member::known)
            .and_then(Number::as_i64),
        Some(1234)
    );
}

#[rstest]
#[case::refused(vec![reply(503, "{}")], 503)]
#[case::unreachable(vec![], 400)]
#[case::missing(vec![reply(200, r#"{"device_auth_id":"id"}"#)], 400)]
#[case::empty_code(vec![reply(200, r#"{"device_auth_id":"id","user_code":""}"#)], 400)]
#[case::not_an_object(vec![reply(200, "[]")], 400)]
#[tokio::test]
async fn device_code_failures_keep_status(#[case] replies: Vec<Reply>, #[case] status: u16) {
    let mut subject = subject(None, 1000, replies);
    let error = subject.login_device_code().await.unwrap_err();
    assert!(matches!(error, LoginError::DeviceCode(_)));
    assert_eq!(error.status(), status);
    assert!(subject.display.prompts.is_empty());
    assert!(subject.store.files.writes.is_empty());
}

#[rstest]
#[case::server_error(vec![reply(200, DEVICE), reply(500, "{}")], 500)]
#[case::unreachable(vec![reply(200, DEVICE), Reply::Unreachable], 400)]
#[case::not_json(vec![reply(200, DEVICE), reply(200, "nope")], 400)]
#[case::redirect(vec![reply(200, DEVICE), reply(302, "")], 302)]
#[tokio::test]
async fn polling_failures_keep_status(#[case] replies: Vec<Reply>, #[case] status: u16) {
    let mut subject = subject(None, 1000, replies);
    let error = subject.login_device_code().await.unwrap_err();
    assert!(matches!(error, LoginError::AccessToken(_)));
    assert_eq!(error.status(), status);
}

#[tokio::test]
async fn incomplete_successful_polls_time_out_after_900_seconds() {
    let mut subject = subject(None, 1000, vec![reply(200, DEVICE)]);
    subject.http.repeat = Some(Reply::Answer(200, PENDING.to_owned()));
    let error = subject.login_device_code().await.unwrap_err();
    assert!(matches!(error, LoginError::AccessToken(Failure::Expired)));
    assert_eq!(error.status(), 408);
    assert_eq!(subject.http.posts.len(), 1 + 900_usize.div_ceil(7));
    assert_eq!(subject.store.clock.now, Decimal::from(1903_i64));
}

#[tokio::test]
async fn other_success_statuses_wait_like_pending() {
    let fresh = jwt(4_000_000_000);
    let replies = vec![
        reply(200, DEVICE),
        reply(202, ""),
        reply(200, AUTHORIZED),
        reply(200, &tokens(&fresh, Some("r"), &fresh)),
    ];
    let mut subject = subject(None, 1000, replies);
    subject.login_device_code().await.unwrap();
    assert_eq!(subject.store.clock.pauses.len(), 1);
}

#[tokio::test]
async fn exchange_body_is_the_literal_unescaped_form() {
    let fresh = jwt(4_000_000_000);
    let replies = vec![
        reply(200, DEVICE),
        reply(200, AUTHORIZED),
        reply(200, &tokens(&fresh, Some("r"), &fresh)),
    ];
    let mut subject = subject(None, 1000, replies);
    subject.login_device_code().await.unwrap();
    let post = &subject.http.posts[2];
    assert_eq!(post.url, OAUTH_TOKEN_URL);
    assert!(!post.json);
    assert_eq!(
        post.body,
        "grant_type=authorization_code&code=auth&code&redirect_uri=https://auth.openai.com/deviceauth/callback&client_id=app_EMoamEEZ73f0CkXaXp7hrann&code_verifier=verifier"
    );
}

#[rstest]
#[case::refused(reply(401, "{}"), 401)]
#[case::no_refresh_token(reply(200, r#"{"access_token":"a","id_token":"i"}"#), 400)]
#[case::no_id_token(reply(200, r#"{"access_token":"a","refresh_token":"r"}"#), 400)]
#[tokio::test]
async fn exchange_failures_keep_status(#[case] exchange: Reply, #[case] status: u16) {
    let mut subject = subject(
        None,
        1000,
        vec![reply(200, DEVICE), reply(200, AUTHORIZED), exchange],
    );
    let error = subject.login_device_code().await.unwrap_err();
    assert!(matches!(error, LoginError::AccessToken(_)));
    assert_eq!(error.status(), status);
    assert_eq!(subject.store.files.writes.len(), 1);
}

#[tokio::test]
async fn concurrent_login_in_cooldown_returns_the_other_token() {
    let access = jwt(4_000_000_000);
    let pending = br#"{"device_code_requested_at":990}"#.to_vec();
    let mut subject = subject(Some(pending), 1000, vec![]);
    subject.store.clock.arrival = Some((
        Arc::clone(&subject.store.files.content),
        record(&access, "r", 4_000_000_000),
    ));
    let token = subject.get_access_token().await.unwrap();
    assert_eq!(token.expose_secret(), access);
    assert_eq!(subject.store.clock.sleeps, [Duration::from_secs(5)]);
    assert!(subject.http.posts.is_empty());
}

#[tokio::test]
async fn device_login_rechecks_the_cooldown_before_requesting_a_code() {
    let access = jwt(4_000_000_000);
    let pending = br#"{"device_code_requested_at":995}"#.to_vec();
    let mut subject = subject(Some(pending), 1000, vec![]);
    subject.store.clock.arrival = Some((
        Arc::clone(&subject.store.files.content),
        record(&access, "r", 4_000_000_000),
    ));
    let token = subject.login_device_code().await.unwrap();
    assert_eq!(token.expose_secret(), access);
    assert!(subject.http.posts.is_empty());
}

#[tokio::test]
async fn expired_cooldown_without_a_token_requests_a_code() {
    let pending = br#"{"device_code_requested_at":990}"#.to_vec();
    let mut subject = subject(Some(pending), 1000, vec![reply(503, "{}")]);
    let error = subject.get_access_token().await.unwrap_err();
    assert_eq!(error.status(), 503);
    let waited: Duration = subject.store.clock.sleeps.iter().sum();
    assert_eq!(waited, Duration::from_secs(290));
    assert_eq!(subject.http.posts.len(), 1);
}

#[tokio::test]
async fn far_future_cooldown_waits_no_longer_than_the_cooldown() {
    let pending = br#"{"device_code_requested_at":900000}"#.to_vec();
    let mut subject = subject(Some(pending), 1000, vec![reply(503, "{}")]);
    subject.get_access_token().await.unwrap_err();
    let waited: Duration = subject.store.clock.sleeps.iter().sum();
    // the wait runs once in get_access_token and once in login_device_code, each capped at the cooldown
    assert_eq!(waited, Duration::from_secs(600));
}

#[tokio::test]
async fn cancelling_a_poll_stops_requests() {
    let mut subject = subject(None, 1000, vec![reply(200, DEVICE), reply(403, "")]);
    subject.store.clock.hang = true;
    let mut attempt = Box::pin(subject.login_device_code());
    assert!(attempt.as_mut().now_or_never().is_none());
    drop(attempt);
    assert_eq!(subject.http.posts.len(), 2);
    assert_eq!(subject.display.prompts.len(), 1);
    assert!(subject.store.clock.pauses.is_empty());
}

#[tokio::test]
async fn diagnostics_hold_no_token_or_code() {
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(log.reopen().unwrap())
        .finish();
    let _guard = set_default(subscriber);
    let fresh = jwt(4_000_000_000);
    let replies = vec![
        reply(401, r#"{"error":"leaked-refresh-body"}"#),
        reply(200, DEVICE),
        reply(200, AUTHORIZED),
        reply(200, &tokens(&fresh, Some("new-refresh"), &fresh)),
    ];
    let mut subject = subject(
        Some(record("stale-access", "old-refresh", 10)),
        1000,
        replies,
    );
    subject.get_access_token().await.unwrap();
    let output = fs::read_to_string(log.path()).unwrap();
    assert!(output.contains("ChatGPT refresh token failed, re-login required"));
    let leaked: Vec<&str> = [
        "old-refresh",
        "stale-access",
        "new-refresh",
        &fresh,
        "ABCD-1234",
        "device-secret",
        "verifier",
        "auth&code",
        "leaked-refresh-body",
    ]
    .into_iter()
    .filter(|secret| output.contains(secret))
    .collect();
    assert_eq!(leaked, Vec::<&str>::new());
}

#[rstest]
#[case::absent("{}", 5)]
#[case::string(r#"{"interval":"12"}"#, 12)]
#[case::padded_string(r#"{"interval":" 12 "}"#, 12)]
#[case::number(r#"{"interval":9}"#, 9)]
#[case::zero(r#"{"interval":0}"#, 5)]
#[case::false_flag(r#"{"interval":false}"#, 5)]
#[case::null(r#"{"interval":null}"#, 5)]
#[case::empty(r#"{"interval":""}"#, 5)]
#[case::below_minimum(r#"{"interval":2}"#, 5)]
#[case::negative(r#"{"interval":-3}"#, 5)]
#[case::beyond_window(r#"{"interval":100000}"#, 900)]
fn device_interval_forms(#[case] interval: &str, #[case] seconds: u64) {
    let opened = interval.replacen('{', r#"{"device_auth_id":"id","user_code":"code","#, 1);
    let body = opened.replace(",}", "}");
    let code = decode_chatgpt_oauth::<DeviceReply>(body.as_bytes())
        .unwrap()
        .into_code()
        .unwrap();
    assert_eq!(code.interval, Duration::from_secs(seconds));
}

#[rstest]
#[case::text(r#"{"interval":"soon"}"#)]
#[case::fraction(r#"{"interval":2.5}"#)]
#[case::true_flag(r#"{"interval":true}"#)]
fn device_interval_rejects_non_integers(#[case] interval: &str) {
    let body = interval.replacen('{', r#"{"device_auth_id":"id","user_code":"code","#, 1);
    let outcome =
        decode_chatgpt_oauth::<DeviceReply>(body.as_bytes()).and_then(DeviceReply::into_code);
    assert!(matches!(
        outcome,
        Err(DecodeError::Interval | DecodeError::Json(_))
    ));
}

#[test]
fn user_code_prefers_user_code_over_usercode() {
    let body = br#"{"device_auth_id":"id","user_code":"first","usercode":"second"}"#;
    let code = decode_chatgpt_oauth::<DeviceReply>(body)
        .unwrap()
        .into_code()
        .unwrap();
    assert_eq!(code.user_code.expose_secret(), "first");
}

#[rstest]
#[case::authorization_code(r#"{"authorization_code":"","code_challenge":"c","code_verifier":"v"}"#)]
#[case::code_challenge(r#"{"authorization_code":"a","code_verifier":"v"}"#)]
#[case::code_verifier(r#"{"authorization_code":"a","code_challenge":"c","code_verifier":null}"#)]
fn authorization_needs_all_three_fields(#[case] body: &str) {
    let reply = decode_chatgpt_oauth::<AuthorizationReply>(body.as_bytes()).unwrap();
    assert!(reply.complete().is_none());
}

#[test]
fn replies_over_the_limit_are_refused() {
    let padding = " ".repeat(REPLY_LIMIT - 2);
    let at_limit = format!("{{}}{padding}");
    decode_chatgpt_oauth::<TokenReply>(at_limit.as_bytes()).unwrap();
    let over = format!("{at_limit} ");
    assert!(matches!(
        decode_chatgpt_oauth::<TokenReply>(over.as_bytes()),
        Err(DecodeError::Size)
    ));
}

#[rstest]
fn replays_oauth_reply(#[files("fuzz/corpus/oauth_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _device = decode_chatgpt_oauth::<DeviceReply>(&bytes).map(DeviceReply::into_code);
    let _authorization =
        decode_chatgpt_oauth::<AuthorizationReply>(&bytes).map(AuthorizationReply::complete);
    let _tokens = decode_chatgpt_oauth::<TokenReply>(&bytes).map(TokenReply::exchanged);
}
