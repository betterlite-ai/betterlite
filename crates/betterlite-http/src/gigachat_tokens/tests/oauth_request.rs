use std::future::pending;
use std::time::Duration;

use http::StatusCode;
use reqwest::Method;
use reqwest::redirect::Policy;
use rstest::rstest;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::gigachat_tokens::authenticator::{GigachatAuthenticator, oauth_request};
use crate::gigachat_tokens::error::GigachatAuthError;
use crate::gigachat_tokens::reply::MAX_REPLY_BYTES;
use crate::gigachat_tokens::request::GigachatTokenRequest;
use crate::gigachat_tokens::runtime::{GigachatRuntime, SystemRuntime};
use crate::gigachat_tokens::tests::support;
use crate::gigachat_tokens::tokens::GigachatTokens as _;

const CREDENTIALS: &str = "Y2xpZW50OnNlY3JldA==";
const TOKEN_REPLY: &str = "{\"tok\":\"fetched-token\",\"exp\":4102444800000}";

fn request(auth_url: &str) -> GigachatTokenRequest {
    GigachatTokenRequest {
        access_token: None,
        credentials: None,
        scope: None,
        auth_url: Some(auth_url.to_owned()),
    }
}

fn authenticator<R>(runtime: R, auth_url: &str) -> GigachatAuthenticator<R>
where
    R: GigachatRuntime,
{
    let settings = support::settings(&[
        ("GIGACHAT_CREDENTIALS", CREDENTIALS),
        ("GIGACHAT_SCOPE", "GIGACHAT_API_PERS"),
        ("GIGACHAT_AUTH_URL", auth_url),
    ]);
    GigachatAuthenticator::new(settings, support::client(), runtime)
}

#[tokio::test]
async fn oauth_request_posts_the_scope_form_to_the_auth_url() {
    let server = support::Server::start().await;
    server.answer(200, TOKEN_REPLY);
    let auth_url = format!("{}/oauth", server.url);
    let auth = authenticator(support::ScriptedRuntime::new(&[0]), &auth_url);

    let token = auth
        .access_token(&request(&auth_url))
        .await
        .expect("the OAuth server answers a token");

    assert_eq!(support::text(&token), "fetched-token");
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let seen = &requests[0];
    assert_eq!(seen.method, "POST");
    assert_eq!(seen.path, "/oauth");
    assert_eq!(seen.header("authorization"), format!("Basic {CREDENTIALS}"));
    assert_eq!(seen.header("rquid"), support::request_uid().to_string());
    assert_eq!(
        seen.header("content-type"),
        "application/x-www-form-urlencoded"
    );
    assert_eq!(seen.body, "scope=GIGACHAT_API_PERS");
}

#[tokio::test]
async fn oauth_request_takes_the_request_scope_before_the_settings_scope() {
    let server = support::Server::start().await;
    server.answer(200, TOKEN_REPLY);
    let auth = authenticator(SystemRuntime, &server.url);
    let mut supplied = request(&server.url);
    supplied.scope = Some("GIGACHAT_API_CORP".to_owned());

    auth.access_token(&supplied)
        .await
        .expect("the OAuth server answers a token");

    let requests = server.requests();
    assert_eq!(requests[0].body, "scope=GIGACHAT_API_CORP");
}

#[tokio::test]
async fn oauth_request_skips_an_empty_request_scope() {
    let server = support::Server::start().await;
    server.answer(200, TOKEN_REPLY);
    let auth = authenticator(SystemRuntime, &server.url);
    let mut supplied = request(&server.url);
    supplied.scope = Some(String::new());

    auth.access_token(&supplied)
        .await
        .expect("the OAuth server answers a token");

    let requests = server.requests();
    assert_eq!(requests[0].body, "scope=GIGACHAT_API_PERS");
}

#[tokio::test]
async fn oauth_request_takes_the_request_auth_url_before_the_settings() {
    let server = support::Server::start().await;
    server.answer(200, TOKEN_REPLY);
    let settings = support::settings(&[
        ("GIGACHAT_CREDENTIALS", CREDENTIALS),
        (
            "GIGACHAT_AUTH_URL",
            format!("{}/settings", server.url).as_str(),
        ),
    ]);
    let auth = GigachatAuthenticator::new(settings, support::client(), SystemRuntime);
    let request_url = format!("{}/request", server.url);

    auth.access_token(&request(&request_url))
        .await
        .expect("the OAuth server answers a token");

    let requests = server.requests();
    assert_eq!(requests[0].path, "/request");
}

#[tokio::test]
async fn oauth_request_skips_an_empty_request_auth_url() {
    let server = support::Server::start().await;
    server.answer(200, TOKEN_REPLY);
    let settings = support::settings(&[
        ("GIGACHAT_CREDENTIALS", CREDENTIALS),
        (
            "GIGACHAT_AUTH_URL",
            format!("{}/settings", server.url).as_str(),
        ),
    ]);
    let auth = GigachatAuthenticator::new(settings, support::client(), SystemRuntime);
    let mut supplied = request("");
    supplied.auth_url = Some(String::new());

    auth.access_token(&supplied)
        .await
        .expect("the OAuth server answers a token");

    let requests = server.requests();
    assert_eq!(requests[0].path, "/settings");
}

#[rstest]
#[case::bad_request(400, "denied")]
#[case::unauthorized(401, "denied")]
#[case::server_error(500, "denied")]
#[tokio::test]
async fn oauth_request_maps_a_non_success_status_to_rejected(
    #[case] status: u16,
    #[case] body: &str,
) {
    let server = support::Server::start().await;
    server.answer(status, body);
    let auth = authenticator(SystemRuntime, &server.url);

    let error = auth
        .access_token(&request(&server.url))
        .await
        .expect_err("a non-2xx status is refused");

    let GigachatAuthError::Rejected {
        status: found,
        body: kept,
    } = error
    else {
        panic!("a non-2xx status becomes a Rejected error");
    };
    assert_eq!(found.as_u16(), status);
    assert_eq!(kept, body);
    assert_eq!(found, StatusCode::from_u16(status).unwrap());
}

#[tokio::test]
async fn oauth_request_does_not_follow_a_redirect() {
    let server = support::Server::start().await;
    let elsewhere = support::Server::start().await;
    elsewhere.answer(200, TOKEN_REPLY);
    server.answer_redirect(302, &format!("{}/next", elsewhere.url));
    let auth = authenticator(SystemRuntime, &server.url);

    let error = auth
        .access_token(&request(&server.url))
        .await
        .expect_err("a redirect is not followed");

    let GigachatAuthError::Rejected { status, .. } = error else {
        panic!("a 302 becomes a Rejected error");
    };
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(server.requests().len(), 1);
    assert!(elsewhere.requests().is_empty());
}

#[tokio::test]
async fn oauth_request_maps_a_connection_failure_to_transport() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port opens");
    let address = listener.local_addr().expect("the bound address is known");
    drop(listener);
    let auth_url = format!("http://{address}/oauth");
    let auth = authenticator(SystemRuntime, &auth_url);

    let error = auth
        .access_token(&request(&format!("http://{address}/oauth")))
        .await
        .expect_err("a closed port refuses the request");

    assert!(matches!(&error, GigachatAuthError::Transport { .. }));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn oauth_request_keeps_at_most_1024_bytes_of_the_rejected_body() {
    let server = support::Server::start().await;
    let body = "x".repeat(2000);
    server.answer(400, &body);
    let auth = authenticator(SystemRuntime, &server.url);

    let error = auth
        .access_token(&request(&server.url))
        .await
        .expect_err("a non-2xx status is refused");

    let GigachatAuthError::Rejected { body: kept, .. } = error else {
        panic!("a non-2xx status becomes a Rejected error");
    };
    assert_eq!(kept.len(), 1024);
    assert_eq!(kept, "x".repeat(1024));
}

#[tokio::test]
async fn oauth_request_keeps_the_text_up_to_an_invalid_byte() {
    let server = support::Server::start().await;
    server.answer_bytes(400, b"denied\xffmore");
    let auth = authenticator(SystemRuntime, &server.url);

    let error = auth
        .access_token(&request(&server.url))
        .await
        .expect_err("a non-2xx status is refused");

    let GigachatAuthError::Rejected { body, .. } = error else {
        panic!("a non-2xx status becomes a Rejected error");
    };
    assert_eq!(body, "denied");
}

#[tokio::test]
async fn oauth_request_rejects_an_unusable_success_reply() {
    let server = support::Server::start().await;
    server.answer(200, "[]");
    let auth = authenticator(SystemRuntime, &server.url);

    let error = auth
        .access_token(&request(&server.url))
        .await
        .expect_err("a non-object reply is refused");

    assert!(matches!(&error, GigachatAuthError::InvalidReply));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn oauth_request_reads_a_reply_of_exactly_64_kib() {
    let server = support::Server::start().await;
    let mut body = String::from("{\"tok\":\"fetched-token\"}");
    body.push_str(&" ".repeat(MAX_REPLY_BYTES.saturating_sub(body.len())));
    assert_eq!(body.len(), MAX_REPLY_BYTES);
    server.answer(200, &body);
    let auth = authenticator(SystemRuntime, &server.url);

    let token = auth
        .access_token(&request(&server.url))
        .await
        .expect("a body of exactly the limit is read");

    assert_eq!(support::text(&token), "fetched-token");
}

#[tokio::test]
async fn oauth_request_rejects_a_reply_over_64_kib() {
    let server = support::Server::start().await;
    let mut body = String::from("{\"tok\":\"fetched-token\"}");
    body.push_str(&" ".repeat(MAX_REPLY_BYTES.saturating_add(1).saturating_sub(body.len())));
    server.answer(200, &body);
    let auth = authenticator(SystemRuntime, &server.url);

    let error = auth
        .access_token(&request(&server.url))
        .await
        .expect_err("a body over the limit is refused");

    assert!(matches!(&error, GigachatAuthError::InvalidReply));
}

#[test]
fn oauth_request_carries_a_thirty_second_timeout() {
    let client = support::client();
    let auth_url = "http://127.0.0.1:9/oauth";
    let request = oauth_request(
        &client,
        auth_url,
        &SecretString::from("Basic-credentials"),
        support::request_uid(),
        "GIGACHAT_API_PERS",
    )
    .expect("the OAuth request builds");

    assert_eq!(request.method(), Method::POST);
    assert_eq!(request.url().as_str(), auth_url);
    assert_eq!(request.timeout(), Some(&Duration::from_secs(30)));
    let header = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    assert_eq!(header("authorization"), Some("Basic Basic-credentials"));
    let uid = support::request_uid().to_string();
    assert_eq!(header("rquid"), Some(uid.as_str()));
    assert_eq!(
        header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        request.body().and_then(reqwest::Body::as_bytes),
        Some(&b"scope=GIGACHAT_API_PERS"[..])
    );
    assert!(!format!("{request:?}").contains("Basic-credentials"));
}

#[tokio::test]
async fn oauth_request_reports_a_timeout_with_status_408() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port opens");
    let address = listener.local_addr().expect("the bound address is known");
    let mut held = JoinSet::new();
    held.spawn(async move {
        if let Ok((_stream, _)) = listener.accept().await {
            pending::<()>().await;
        }
    });
    let auth_url = format!("http://{address}/oauth");
    let settings = support::settings(&[
        ("GIGACHAT_CREDENTIALS", CREDENTIALS),
        ("GIGACHAT_AUTH_URL", auth_url.as_str()),
    ]);
    let client = reqwest::ClientBuilder::new()
        .no_proxy()
        .redirect(Policy::none())
        .read_timeout(Duration::from_millis(50))
        .build()
        .expect("the test HTTP client builds");
    let auth = GigachatAuthenticator::new(settings, client, SystemRuntime);

    let error = auth
        .access_token(&request(&auth_url))
        .await
        .expect_err("a server that never answers times out");

    assert!(matches!(
        &error,
        GigachatAuthError::Transport { source } if source.is_timeout()
    ));
    assert_eq!(error.status_code(), StatusCode::REQUEST_TIMEOUT);
}

#[tokio::test]
async fn oauth_request_mints_a_new_uid_for_each_request() {
    let server = support::Server::start().await;
    server.answer(200, TOKEN_REPLY);
    server.answer(200, TOKEN_REPLY);
    let auth = authenticator(SystemRuntime, &server.url);
    let mut first = request(&server.url);
    first.credentials = Some(SecretString::from("AAAAAAAAAAAAAAAA-one"));
    let mut second = request(&server.url);
    second.credentials = Some(SecretString::from("BBBBBBBBBBBBBBBB-two"));

    auth.access_token(&first)
        .await
        .expect("the first request is answered");
    auth.access_token(&second)
        .await
        .expect("the second request is answered");

    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    let first_uid = Uuid::parse_str(requests[0].header("rquid")).expect("the first id is a UUID");
    let second_uid = Uuid::parse_str(requests[1].header("rquid")).expect("the second id is a UUID");
    assert_ne!(first_uid, second_uid);
    assert!(!first_uid.is_nil());
}

#[test]
fn oauth_request_reads_the_system_clock() {
    let now = SystemRuntime.now_millis();

    assert!(now > 1_700_000_000_000);
    assert!(now < 4_000_000_000_000);
}

#[tokio::test]
async fn oauth_request_logs_the_request_and_the_success() {
    let (logs, _guard) = support::Logs::capture();
    let server = support::Server::start().await;
    server.answer(200, TOKEN_REPLY);
    let auth = authenticator(SystemRuntime, &server.url);

    auth.access_token(&request(&server.url))
        .await
        .expect("the OAuth server answers a token");

    let text = logs.text();
    assert!(text.contains(&format!(
        "Requesting GigaChat access token from {}",
        server.url
    )));
    assert!(text.contains("GigaChat access token obtained successfully"));
}

#[tokio::test]
async fn oauth_request_error_text_hides_the_credentials_and_token() {
    let server = support::Server::start().await;
    server.answer(200, "{\"tok\":\"secret-token-value\"}");
    server.answer(400, "denied");
    let auth = authenticator(SystemRuntime, &server.url);
    let mut supplied = request(&server.url);
    supplied.credentials = Some(SecretString::from("secret-credential-value"));

    let token = auth
        .access_token(&supplied)
        .await
        .expect("the OAuth server answers a token");
    let error = auth
        .access_token(&supplied)
        .await
        .expect_err("a non-2xx status is refused");

    let printed_request = format!("{supplied:?}");
    assert!(!printed_request.contains("secret-credential-value"));
    assert!(!format!("{token:?}").contains("secret-token-value"));
    let rejected = format!("{error} {error:?}");
    assert!(!rejected.contains("secret-credential-value"));
    assert!(!rejected.contains("secret-token-value"));
}
