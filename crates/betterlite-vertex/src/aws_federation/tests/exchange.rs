#![cfg(test)]

use std::collections::BTreeMap;
use std::mem;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use url::form_urlencoded;

use super::{
    AUDIENCE, GOLDEN_TOKEN, RecordingResolver, SUBJECT_TOKEN_TYPE, aws_params, credentials_with,
    fixed_now, input,
};
use crate::aws_federation::exchange::{TOKEN_URL_TEMPLATE, access_token, token_endpoint};
use crate::aws_federation::{AwsFederation, FederationError, credentials_from_explicit_aws};

const CLOUD_PLATFORM: &str = "https://www.googleapis.com/auth/cloud-platform";
const IAM: &str = "https://www.googleapis.com/auth/iam";
const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const REQUESTED_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:access_token";
const IMPERSONATION_PATH: &str =
    "/v1/projects/-/serviceAccounts/sa@example.iam.gserviceaccount.com:generateAccessToken";
const STS_ANSWER: &str = r#"{"access_token":"ya29.sts-token","expires_in":3599}"#;
const IAM_ANSWER: &str =
    r#"{"accessToken":"ya29.impersonated","expireTime":"2026-01-01T01:00:00Z"}"#;
const FIXED_NOW_SECONDS: u64 = 1_767_225_600;

struct Recorded {
    request_line: String,
    headers: BTreeMap<String, String>,
    body: String,
}

struct Upstream {
    url: String,
    requests: Arc<Mutex<Vec<Recorded>>>,
    _tasks: JoinSet<()>,
}

async fn serve(mut socket: TcpStream, status: u16, body: &str, requests: &Mutex<Vec<Recorded>>) {
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(socket.read_u8().await.unwrap());
    }
    let head = String::from_utf8(received).unwrap();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap().to_owned();
    let headers: BTreeMap<String, String> = lines
        .filter_map(|line| line.split_once(": "))
        .map(|(name, value)| (name.to_lowercase(), value.to_owned()))
        .collect();
    let mut content = vec![0; headers["content-length"].parse().unwrap()];
    socket.read_exact(&mut content).await.unwrap();
    requests.lock().unwrap().push(Recorded {
        request_line,
        headers,
        body: String::from_utf8(content).unwrap(),
    });
    let reply = format!(
        "HTTP/1.1 {status} Reply\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(reply.as_bytes()).await.unwrap();
}

async fn upstream(replies: &[(u16, &str)]) -> Upstream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&requests);
    let scripted: Vec<(u16, String)> = replies
        .iter()
        .map(|(status, body)| (*status, (*body).to_owned()))
        .collect();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        for (status, body) in scripted {
            let (socket, _) = listener.accept().await.unwrap();
            serve(socket, status, &body, &log).await;
        }
    });
    Upstream {
        url,
        requests,
        _tasks: tasks,
    }
}

fn client() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::outbound(&policy).unwrap()
}

fn federation<'resolver>(
    resolver: &'resolver RecordingResolver,
    upstream: &Upstream,
    impersonation_path: Option<&str>,
    scopes: &[&str],
) -> AwsFederation<'resolver, RecordingResolver> {
    let mut federation_input = input(aws_params(Some("us-east-1")));
    federation_input.token_url = Some(format!("{}/v1/token", upstream.url));
    federation_input.impersonation_url =
        impersonation_path.map(|path| format!("{}{path}", upstream.url));
    federation_input.scopes = scopes.iter().map(|scope| (*scope).to_owned()).collect();
    credentials_from_explicit_aws(federation_input, resolver).unwrap()
}

fn resolver() -> RecordingResolver {
    RecordingResolver::new(vec![credentials_with(
        "AKIAEXAMPLE",
        "secret/example",
        Some("session-token-example"),
    )])
}

fn form(request: &Recorded) -> Vec<(String, String)> {
    form_urlencoded::parse(request.body.as_bytes())
        .into_owned()
        .collect()
}

fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn after_epoch(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .unwrap()
}

#[rstest]
#[case::unscoped(&[], None)]
#[case::one_scope(&[CLOUD_PLATFORM], Some(CLOUD_PLATFORM))]
#[case::two_scopes(&["scope-a", "scope-b"], Some("scope-a scope-b"))]
#[tokio::test]
async fn sts_exchange(#[case] scopes: &[&str], #[case] expected_scope: Option<&str>) {
    let server = upstream(&[(200, STS_ANSWER)]).await;
    let resolver = resolver();
    let federation = federation(&resolver, &server, None, scopes);
    let token = access_token(&federation, &client(), fixed_now())
        .await
        .unwrap();
    let requests = mem::take(&mut *server.requests.lock().unwrap());
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].request_line, "POST /v1/token HTTP/1.1");
    assert_eq!(
        requests[0].headers["content-type"],
        "application/x-www-form-urlencoded"
    );
    assert!(!requests[0].headers.contains_key("authorization"));
    let mut expected = vec![("grant_type", GRANT_TYPE), ("audience", AUDIENCE)];
    expected.extend(expected_scope.map(|scope| ("scope", scope)));
    expected.extend([
        ("requested_token_type", REQUESTED_TOKEN_TYPE),
        ("subject_token", GOLDEN_TOKEN),
        ("subject_token_type", SUBJECT_TOKEN_TYPE),
    ]);
    assert_eq!(form(&requests[0]), owned(&expected));
    assert_eq!(token.token.expose_secret(), "ya29.sts-token");
    assert_eq!(token.expires_at, after_epoch(FIXED_NOW_SECONDS + 3599));
    assert_eq!(resolver.calls(), 1);
}

#[rstest]
#[case::number(r#"{"access_token":"ya29.sts-token","expires_in":3599}"#)]
#[case::text(r#"{"access_token":"ya29.sts-token","expires_in":"3599"}"#)]
#[case::extra_field(r#"{"access_token":"ya29.sts-token","expires_in":3599,"token_type":"Bearer"}"#)]
#[tokio::test]
async fn sts_lifetime_forms(#[case] answer: &str) {
    let server = upstream(&[(200, answer)]).await;
    let resolver = resolver();
    let federation = federation(&resolver, &server, None, &[]);
    let token = access_token(&federation, &client(), fixed_now())
        .await
        .unwrap();
    assert_eq!(token.expires_at, after_epoch(FIXED_NOW_SECONDS + 3599));
}

#[rstest]
#[case::absent(None, None)]
#[case::empty(Some(""), Some(""))]
#[tokio::test]
async fn sts_form_omits_missing_fields(
    #[case] audience: Option<&str>,
    #[case] subject_token_type: Option<&str>,
) {
    let server = upstream(&[(200, STS_ANSWER)]).await;
    let resolver = resolver();
    let mut federation = federation(&resolver, &server, None, &[]);
    federation.audience = audience.map(str::to_owned);
    federation.subject_token_type = subject_token_type.map(str::to_owned);
    access_token(&federation, &client(), fixed_now())
        .await
        .unwrap();
    let requests = mem::take(&mut *server.requests.lock().unwrap());
    let names: Vec<String> = form(&requests[0])
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(
        names,
        ["grant_type", "requested_token_type", "subject_token"]
    );
}

#[rstest]
#[case::unscoped(&[], r#"{"delegates":null,"scope":null,"lifetime":"3600s"}"#)]
#[case::scoped(
    &[CLOUD_PLATFORM],
    r#"{"delegates":null,"scope":["https://www.googleapis.com/auth/cloud-platform"],"lifetime":"3600s"}"#
)]
#[tokio::test]
async fn impersonation_exchange(#[case] scopes: &[&str], #[case] expected_body: &str) {
    let server = upstream(&[(200, STS_ANSWER), (200, IAM_ANSWER)]).await;
    let resolver = resolver();
    let federation = federation(&resolver, &server, Some(IMPERSONATION_PATH), scopes);
    let token = access_token(&federation, &client(), fixed_now())
        .await
        .unwrap();
    let requests = mem::take(&mut *server.requests.lock().unwrap());
    assert_eq!(requests.len(), 2);
    let sts_scope = form(&requests[0])
        .into_iter()
        .find(|(name, _)| name == "scope")
        .map(|(_, scope)| scope);
    assert_eq!(sts_scope.as_deref(), Some(IAM));
    assert_eq!(
        requests[1].request_line,
        format!("POST {IMPERSONATION_PATH} HTTP/1.1")
    );
    assert_eq!(
        requests[1].headers["authorization"],
        "Bearer ya29.sts-token"
    );
    assert_eq!(requests[1].headers["content-type"], "application/json");
    assert_eq!(requests[1].body, expected_body);
    assert_eq!(token.token.expose_secret(), "ya29.impersonated");
    assert_eq!(token.expires_at, after_epoch(FIXED_NOW_SECONDS + 3600));
    assert_eq!(resolver.calls(), 1);
}

#[rstest]
#[case::no_marker("SERVER/v1/projects/-/serviceAccounts/sa@example.iam.gserviceaccount.com")]
#[case::path_after_marker("SERVER/sa@example.iam.gserviceaccount.com:generateAccessToken/extra")]
#[case::empty_account("SERVER/v1/projects/-/serviceAccounts/:generateAccessToken")]
#[case::no_slash("account:generateAccessToken")]
#[tokio::test]
async fn impersonation_target_checked_first(#[case] endpoint: &str) {
    let server = upstream(&[]).await;
    let resolver = resolver();
    let mut federation = federation(&resolver, &server, None, &[]);
    federation.impersonation_url = Some(endpoint.replace("SERVER", &server.url));
    let error = access_token(&federation, &client(), fixed_now())
        .await
        .unwrap_err();
    assert!(matches!(error, FederationError::ImpersonationTarget));
    assert_eq!(resolver.calls(), 0);
    assert!(server.requests.lock().unwrap().is_empty());
}

#[rstest]
#[case::sts_refused(&[(400, "{}")], "StsStatus { status: 400 }")]
#[case::sts_created(&[(201, STS_ANSWER)], "StsStatus { status: 201 }")]
#[case::sts_not_a_token(&[(200, r#"{"expires_in":3599}"#)], "StsResponse(")]
#[case::sts_without_lifetime(&[(200, r#"{"access_token":"t"}"#)], "StsResponse(")]
#[case::sts_lifetime_overflow(
    &[(200, r#"{"access_token":"t","expires_in":18446744073709551615}"#)],
    "ExpiryOverflow"
)]
#[case::iam_refused(&[(200, STS_ANSWER), (403, "{}")], "ImpersonationStatus { status: 403 }")]
#[case::iam_not_a_token(&[(200, STS_ANSWER), (200, "{}")], "ImpersonationResponse(")]
#[case::iam_bad_expiry(
    &[(200, STS_ANSWER), (200, r#"{"accessToken":"t","expireTime":"2026-01-01 01:00:00"}"#)],
    "ExpireTime("
)]
#[tokio::test]
async fn exchange_failures(#[case] replies: &[(u16, &str)], #[case] expected: &str) {
    let server = upstream(replies).await;
    let resolver = resolver();
    let impersonation = (replies.len() == 2).then_some(IMPERSONATION_PATH);
    let federation = federation(&resolver, &server, impersonation, &[]);
    let error = access_token(&federation, &client(), fixed_now())
        .await
        .unwrap_err();
    assert!(
        format!("{error:?}").starts_with(expected),
        "{error:?} does not start with {expected}"
    );
}

#[rstest]
#[case::template_with_universe(
    Some(TOKEN_URL_TEMPLATE),
    Some("example.com"),
    "https://sts.example.com/v1/token"
)]
#[case::template_without_universe(
    Some(TOKEN_URL_TEMPLATE),
    None,
    "https://sts.googleapis.com/v1/token"
)]
#[case::custom_url_keeps_its_host(
    Some("https://sts.googleapis.com/v1/token"),
    Some("example.com"),
    "https://sts.googleapis.com/v1/token"
)]
#[case::missing(None, None, "TokenUrlRequired")]
#[case::malformed(Some("not a url"), None, "Url { field: \"token_url\"")]
#[tokio::test]
async fn token_endpoint_resolution(
    #[case] token_url: Option<&str>,
    #[case] universe_domain: Option<&str>,
    #[case] expected: &str,
) {
    let server = upstream(&[]).await;
    let resolver = resolver();
    let mut federation = federation(&resolver, &server, None, &[]);
    federation.token_url = token_url.map(str::to_owned);
    federation.universe_domain = universe_domain.map(str::to_owned);
    let resolved = match token_endpoint(&federation) {
        Ok(url) => url.to_string(),
        Err(error) => format!("{error:?}"),
    };
    assert!(
        resolved.starts_with(expected),
        "{resolved} does not start with {expected}"
    );
}
