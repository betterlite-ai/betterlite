#![cfg(test)]

use std::collections::VecDeque;
use std::error::Error as _;
use std::mem;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use std::{fs, io};

use betterlite_config::sections::xai_oauth_settings::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tempfile::TempDir;
use tokio::task::yield_now;

use super::codec::{
    AuthFile, DOCUMENT_LIMIT, DecodeError, Document, Numeric, TokenPayload, decode_tagged,
    decode_xai_oauth,
};
use super::error::XaiOauthError;
use super::files::LocalFiles;
use super::rules::{DISCOVERY_URL, build_auth_record, is_expired, validate_xai_endpoint};
use super::transport::Transport;
use super::{Clock, XaiOauth};

const NOW: i64 = 1000;
const TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";

struct Fixed;

impl Clock for Fixed {
    fn now(&self) -> Decimal {
        Decimal::from(NOW)
    }
}

enum Reply {
    Body(u16, &'static str),
    Fail,
}

#[derive(Debug)]
struct Seen {
    method: String,
    url: String,
    accept: String,
    content_type: String,
    body: String,
}

#[derive(Default)]
struct Script {
    replies: Mutex<VecDeque<Reply>>,
    seen: Mutex<Vec<Seen>>,
}

impl Transport for Script {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        let (content_type, body) = match request.body {
            RequestBody::Bytes { content_type, data } => (
                content_type.to_str().unwrap().to_owned(),
                String::from_utf8(data).unwrap(),
            ),
            RequestBody::Empty => (String::new(), String::new()),
            RequestBody::Json(_) | RequestBody::Multipart { .. } => panic!("unexpected body"),
        };
        self.seen.lock().unwrap().push(Seen {
            method: request.method.to_string(),
            url: request.url.to_string(),
            accept: request.headers["accept"].to_str().unwrap().to_owned(),
            content_type,
            body,
        });
        yield_now().await;
        let reply = self.replies.lock().unwrap().pop_front().unwrap();
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

struct Fixture {
    _dir: TempDir,
    oauth: XaiOauth<LocalFiles, Fixed, Script>,
}

impl Fixture {
    fn new(auth: Option<&str>, replies: Vec<Reply>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            token_dir: dir.path().join("xai").to_str().unwrap().to_owned(),
            auth_file: "auth.json".to_owned(),
            api_base: String::new(),
            deployments: Vec::new(),
        };
        let script = Script {
            replies: Mutex::new(VecDeque::from(replies)),
            seen: Mutex::new(Vec::new()),
        };
        let oauth = XaiOauth::new(settings, LocalFiles, Fixed, script, Duration::from_secs(5));
        let fixture = Self { _dir: dir, oauth };
        if let Some(text) = auth {
            fs::create_dir_all(fixture.oauth.auth_file.parent().unwrap()).unwrap();
            fs::write(&fixture.oauth.auth_file, text).unwrap();
        }
        fixture
    }

    fn file(&self) -> String {
        fs::read_to_string(&self.oauth.auth_file).unwrap()
    }

    fn take_seen(&self) -> Vec<Seen> {
        mem::take(&mut *self.oauth.transport.seen.lock().unwrap())
    }

    fn calls(&self) -> usize {
        self.oauth.transport.seen.lock().unwrap().len()
    }
}

const STALE: &str = r#"{"access_token":"old-access","refresh_token":"old-refresh","token_endpoint":"https://auth.x.ai/oauth2/token","expires_at":1000}"#;
const FRESH_REPLY: &str = r#"{"access_token":"new-access","refresh_token":"new-refresh","id_token":"new-id","expires_in":7200}"#;

#[tokio::test]
async fn refresh_posts_the_form_and_stores_the_record() {
    let fixture = Fixture::new(Some(STALE), vec![Reply::Body(200, FRESH_REPLY)]);
    let token = fixture.oauth.get_access_token().await.unwrap();
    assert_eq!(token.expose_secret(), "new-access");
    let seen = fixture.take_seen();
    let request = seen.first().unwrap();
    assert_eq!(
        (request.method.as_str(), request.url.as_str()),
        ("POST", TOKEN_URL)
    );
    assert_eq!(request.accept, "application/json");
    assert_eq!(request.content_type, "application/x-www-form-urlencoded");
    assert_eq!(
        request.body,
        "grant_type=refresh_token&refresh_token=old-refresh&client_id=b1a00492-073a-47ea-816f-4c329264a828"
    );
    let stored: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&fixture.file()).unwrap();
    assert_eq!(stored["access_token"], "new-access");
    assert_eq!(stored["refresh_token"], "new-refresh");
    assert_eq!(stored["id_token"], "new-id");
    assert_eq!(stored["token_type"], "Bearer");
    assert_eq!(stored["token_endpoint"], TOKEN_URL);
    assert_eq!(stored["expires_at"], 8_200_i64);
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&fixture.oauth.auth_file), 0o600);
    assert_eq!(mode(fixture.oauth.auth_file.parent().unwrap()), 0o700);
    assert_eq!(
        fs::read_dir(fixture.oauth.auth_file.parent().unwrap())
            .unwrap()
            .count(),
        1
    );
}

#[tokio::test]
async fn two_callers_observe_one_refresh() {
    let fixture = Fixture::new(Some(STALE), vec![Reply::Body(200, FRESH_REPLY)]);
    let (first, second) = tokio::join!(
        fixture.oauth.get_access_token(),
        fixture.oauth.get_access_token()
    );
    assert_eq!(first.unwrap().expose_secret(), "new-access");
    assert_eq!(second.unwrap().expose_secret(), "new-access");
    assert_eq!(fixture.calls(), 1);
}

#[tokio::test]
async fn file_rewritten_while_waiting_skips_the_exchange() {
    let fixture = Fixture::new(Some(STALE), vec![]);
    let guard = fixture.oauth.refresh_lock.lock().await;
    let renewed =
        r#"{"access_token":"other-access","refresh_token":"other-refresh","expires_at":5000}"#;
    let (token, ()) = tokio::join!(fixture.oauth.get_access_token(), async {
        fs::write(&fixture.oauth.auth_file, renewed).unwrap();
        drop(guard);
    });
    assert_eq!(token.unwrap().expose_secret(), "other-access");
    assert_eq!(fixture.calls(), 0);
}

#[tokio::test]
async fn unexpired_file_is_returned_without_a_request() {
    let live = r#"{"access_token":"live","expires_at":1121}"#;
    let fixture = Fixture::new(Some(live), vec![]);
    assert_eq!(
        fixture
            .oauth
            .get_access_token()
            .await
            .unwrap()
            .expose_secret(),
        "live"
    );
    assert_eq!(fixture.calls(), 0);
}

#[rstest]
#[case::missing(None)]
#[case::malformed(Some("{"))]
#[case::array(Some("[]"))]
#[case::null(Some("null"))]
#[tokio::test]
async fn unreadable_auth_file_means_login_required(#[case] text: Option<&str>) {
    let fixture = Fixture::new(text, vec![]);
    let error = fixture.oauth.get_access_token().await.unwrap_err();
    assert!(matches!(error, XaiOauthError::LoginRequired), "{error}");
}

#[tokio::test]
async fn oversized_auth_file_means_login_required() {
    let padding = " ".repeat(DOCUMENT_LIMIT);
    let fixture = Fixture::new(Some(&format!("{STALE}{padding}")), vec![]);
    let error = fixture.oauth.get_access_token().await.unwrap_err();
    assert!(matches!(error, XaiOauthError::LoginRequired));
}

#[tokio::test]
async fn expired_file_without_a_refresh_token_asks_for_login() {
    let fixture = Fixture::new(Some(r#"{"access_token":"a","expires_at":1}"#), vec![]);
    let error = fixture.oauth.get_access_token().await.unwrap_err();
    assert!(matches!(error, XaiOauthError::RefreshTokenMissing));
}

#[rstest]
#[case::absent(None, true)]
#[case::null(Some("null"), true)]
#[case::text(Some("\"abc\""), true)]
#[case::list(Some("[]"), true)]
#[case::boundary(Some("1120"), true)]
#[case::below_boundary(Some("1121"), false)]
#[case::fraction(Some("1120.5"), false)]
#[case::numeric_text(Some("\"1121\""), false)]
#[case::numeric_text_boundary(Some("\"1120\""), true)]
#[case::exponent(Some("1.121e3"), false)]
#[case::infinity(Some("\"inf\""), true)]
#[case::not_a_number(Some("\"nan\""), true)]
fn expiry_renews_120_seconds_early(#[case] json: Option<&'static str>, #[case] expired: bool) {
    let value: Option<Numeric> = json.map(|text| serde_json::from_str(text).unwrap());
    assert_eq!(is_expired(Decimal::from(NOW), value.as_ref()), expired);
}

#[rstest]
#[case::plain("https://auth.x.ai/oauth2/token", true)]
#[case::apex("https://x.ai/token", true)]
#[case::deep("https://a.b.x.ai/token", true)]
#[case::uppercase("https://AUTH.X.AI/token", true)]
#[case::port("https://auth.x.ai:8443/token", true)]
#[case::port_out_of_range("https://auth.x.ai:99999/token", false)]
#[case::http("http://auth.x.ai/token", false)]
#[case::suffix("https://evilx.ai/token", false)]
#[case::prefix("https://x.ai.evil.com/token", false)]
#[case::userinfo("https://x.ai@evil.com/token", false)]
#[case::query("https://evil.com/?x.ai", false)]
#[case::no_host("https:///token", false)]
#[case::relative("/token", false)]
fn endpoints_must_be_https_on_x_ai(#[case] endpoint: &str, #[case] allowed: bool) {
    assert_eq!(validate_xai_endpoint(endpoint).is_ok(), allowed);
}

#[tokio::test]
async fn stored_endpoint_off_x_ai_stops_before_any_request() {
    let evil = STALE.replace("auth.x.ai", "evil.example");
    let fixture = Fixture::new(Some(&evil), vec![]);
    let error = fixture.oauth.get_access_token().await.unwrap_err();
    assert!(matches!(error, XaiOauthError::UnexpectedEndpoint));
    assert_eq!(fixture.calls(), 0);
    assert_eq!(fixture.file(), evil);
}

#[tokio::test]
async fn discovery_supplies_the_token_endpoint() {
    let bare = r#"{"access_token":"old","refresh_token":"old-refresh","expires_at":1}"#;
    let discovery = r#"{"authorization_endpoint":"https://auth.x.ai/authorize","token_endpoint":"https://auth.x.ai/discovered"}"#;
    let replies = vec![Reply::Body(200, discovery), Reply::Body(200, FRESH_REPLY)];
    let fixture = Fixture::new(Some(bare), replies);
    fixture.oauth.get_access_token().await.unwrap();
    let seen = fixture.take_seen();
    assert_eq!(
        (seen[0].method.as_str(), seen[0].url.as_str()),
        ("GET", DISCOVERY_URL)
    );
    assert_eq!(seen[0].accept, "application/json");
    assert_eq!(seen[1].url, "https://auth.x.ai/discovered");
    assert!(fixture.file().contains("https://auth.x.ai/discovered"));
}

#[rstest]
#[case::no_token(
    r#"{"authorization_endpoint":"https://auth.x.ai/a"}"#,
    "missing endpoints"
)]
#[case::no_authorization(r#"{"token_endpoint":"https://auth.x.ai/t"}"#, "missing endpoints")]
#[case::empty(
    r#"{"authorization_endpoint":"","token_endpoint":"https://auth.x.ai/t"}"#,
    "missing endpoints"
)]
#[case::foreign_authorization(
    r#"{"authorization_endpoint":"https://evil.com/a","token_endpoint":"https://auth.x.ai/t"}"#,
    "outside https x.ai"
)]
#[case::foreign_token(
    r#"{"authorization_endpoint":"https://auth.x.ai/a","token_endpoint":"http://auth.x.ai/t"}"#,
    "outside https x.ai"
)]
#[tokio::test]
async fn discovery_needs_two_endpoints_on_x_ai(#[case] reply: &'static str, #[case] message: &str) {
    let bare = r#"{"access_token":"old","refresh_token":"old-refresh","expires_at":1}"#;
    let fixture = Fixture::new(Some(bare), vec![Reply::Body(200, reply)]);
    let error = fixture.oauth.get_access_token().await.unwrap_err();
    assert!(error.to_string().contains(message), "{error}");
    assert_eq!(fixture.file(), bare);
}

fn payload(json: &str) -> TokenPayload {
    decode_xai_oauth(json.as_bytes()).unwrap()
}

#[rstest]
#[case::missing_field(r#"{"access_token":"a","refresh_token":"r"}"#, 4600)]
#[case::zero(r#"{"access_token":"a","refresh_token":"r","expires_in":0}"#, 4600)]
#[case::null(r#"{"access_token":"a","refresh_token":"r","expires_in":null}"#, 4600)]
#[case::text_zero(r#"{"access_token":"a","refresh_token":"r","expires_in":"0"}"#, 1000)]
#[case::numeric_text(
    r#"{"access_token":"a","refresh_token":"r","expires_in":"7200"}"#,
    8200
)]
#[case::fraction(r#"{"access_token":"a","refresh_token":"r","expires_in":1.9}"#, 1001)]
#[case::invalid_text(r#"{"access_token":"a","refresh_token":"r","expires_in":"abc"}"#, 4600)]
#[case::empty_text(r#"{"access_token":"a","refresh_token":"r","expires_in":""}"#, 4600)]
#[case::list(r#"{"access_token":"a","refresh_token":"r","expires_in":[]}"#, 4600)]
fn lifetime_falls_back_to_one_hour(#[case] json: &str, #[case] expires_at: i64) {
    let record = build_auth_record(Decimal::from(NOW), payload(json), TOKEN_URL, None).unwrap();
    assert_eq!(record.expires_at, expires_at);
    assert_eq!(record.token_type, "Bearer");
}

#[rstest]
#[case::keeps_old_refresh_token(r#"{"access_token":"a"}"#, Some("kept"), Some("kept"))]
#[case::prefers_new_refresh_token(
    r#"{"access_token":"a","refresh_token":"new"}"#,
    Some("kept"),
    Some("new")
)]
#[case::empty_new_refresh_token(
    r#"{"access_token":"a","refresh_token":""}"#,
    Some("kept"),
    Some("kept")
)]
#[case::no_refresh_token(r#"{"access_token":"a"}"#, None, None)]
#[case::missing_access_token(r#"{"refresh_token":"r"}"#, Some("kept"), None)]
#[case::empty_access_token(r#"{"access_token":"","refresh_token":"r"}"#, Some("kept"), None)]
fn record_refresh_token_and_required_fields(
    #[case] json: &str,
    #[case] fallback: Option<&str>,
    #[case] refresh: Option<&str>,
) {
    let kept = fallback.map(SecretString::from);
    let built = build_auth_record(Decimal::from(NOW), payload(json), TOKEN_URL, kept.as_ref());
    match (built, refresh) {
        (Ok(record), Some(expected)) => assert_eq!(record.refresh_token.expose_secret(), expected),
        (Err(XaiOauthError::MissingRefreshToken | XaiOauthError::MissingAccessToken), None) => {}
        (other, _) => panic!("unexpected {other:?}"),
    }
}

#[rstest]
#[case::transport(Reply::Fail)]
#[case::status(Reply::Body(400, FRESH_REPLY))]
#[case::invalid_json(Reply::Body(200, "nope"))]
#[case::array(Reply::Body(200, "[]"))]
#[case::no_access_token(Reply::Body(200, r#"{"refresh_token":"r"}"#))]
#[tokio::test]
async fn failed_exchange_leaves_the_file_alone(#[case] reply: Reply) {
    let fixture = Fixture::new(Some(STALE), vec![reply]);
    let error = fixture.oauth.get_access_token().await.unwrap_err();
    assert_eq!(fixture.file(), STALE);
    assert_eq!(
        fs::read_dir(fixture.oauth.auth_file.parent().unwrap())
            .unwrap()
            .count(),
        1
    );
    assert!(!format!("{error} {error:?}").contains("new-access"));
    assert!(!format!("{error:?}").contains("old-refresh"));
}

#[test]
fn failed_write_removes_the_temp_file_and_keeps_the_cause() {
    let fixture = Fixture::new(None, vec![]);
    fs::create_dir_all(fixture.oauth.auth_file.join("occupied")).unwrap();
    let record =
        build_auth_record(Decimal::from(NOW), payload(FRESH_REPLY), TOKEN_URL, None).unwrap();
    let error = fixture.oauth.write_auth_file(&record).unwrap_err();
    assert!(matches!(error, XaiOauthError::Write(_)));
    let cause = error.source().unwrap().downcast_ref::<io::Error>().unwrap();
    assert_ne!(cause.kind(), io::ErrorKind::NotFound);
    let names: Vec<_> = fs::read_dir(fixture.oauth.auth_file.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["auth.json"]);
}

#[rstest]
#[case::new_access("new-access")]
#[case::new_refresh("new-refresh")]
#[case::new_id("new-id")]
#[case::old_access("old-access")]
#[case::old_refresh("old-refresh")]
#[case::partial_document("leaky-secret")]
fn debug_output_hides_every_secret(#[case] secret: &str) {
    let record =
        build_auth_record(Decimal::from(NOW), payload(FRESH_REPLY), TOKEN_URL, None).unwrap();
    let auth: AuthFile = decode_xai_oauth(STALE.as_bytes()).unwrap();
    let debug = format!("{record:?}{auth:?}{:?}", payload(FRESH_REPLY));
    let error = XaiOauthError::Response {
        stage: "token",
        source: decode_xai_oauth::<TokenPayload>(b"{\"access_token\":\"leaky-secret\"")
            .unwrap_err(),
    };
    let shown = format!("{debug}{error}{error:?}");
    assert!(!shown.contains(secret));
}

#[test]
fn tagged_documents_pick_their_type() {
    assert!(matches!(decode_tagged(b"\x00{}"), Ok(Document::Auth(_))));
    assert!(matches!(
        decode_tagged(b"\x01{}"),
        Ok(Document::Discovery(_))
    ));
    assert!(matches!(decode_tagged(b"\x02{}"), Ok(Document::Token(_))));
    assert!(matches!(decode_tagged(b"\x03{}"), Err(DecodeError::Tag)));
    assert!(matches!(decode_tagged(b""), Err(DecodeError::Tag)));
    let oversized = [&[0_u8][..], &vec![b' '; DOCUMENT_LIMIT + 1]].concat();
    assert!(matches!(decode_tagged(&oversized), Err(DecodeError::Size)));
}

#[rstest]
fn replays_xai_oauth_corpus(#[files("fuzz/corpus/xai_oauth/*")] path: PathBuf) {
    let _document = decode_tagged(&fs::read(path).unwrap());
}
