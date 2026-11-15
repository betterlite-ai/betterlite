use std::fs;
use std::str::from_utf8;
use std::time::Duration;

use crate::databricks_auth::oauth::{
    FORM_CONTENT_TYPE, OAUTH_TIMEOUT, OauthReply, OauthTokenRequest, OauthTokenSource,
    oauth_token_request, parse_oauth_token_reply, request_oauth_token,
};
use async_trait::async_trait;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Builder;

struct FixedSource(OauthReply);

#[async_trait]
impl OauthTokenSource for FixedSource {
    async fn send(&self, _request: &OauthTokenRequest) -> OauthReply {
        self.0.clone()
    }
}

#[test]
fn the_token_request_names_the_workspace_and_the_form() {
    let request = oauth_token_request(
        "https://adb-1.databricks.net/serving-endpoints",
        "client-id",
        "client-secret",
    );
    assert_eq!(request.url, "https://adb-1.databricks.net/oidc/v1/token");
    assert_eq!(request.body, "grant_type=client_credentials&scope=all-apis");
    assert_eq!(
        request.authorization,
        "Basic Y2xpZW50LWlkOmNsaWVudC1zZWNyZXQ="
    );
    assert_eq!(request.content_type, FORM_CONTENT_TYPE);
    assert_eq!(request.timeout, OAUTH_TIMEOUT);
    assert_eq!(request.timeout, Duration::from_secs(30));
}

#[test]
fn the_token_request_does_not_print_its_credential() {
    let request = oauth_token_request("https://host", "client-id", "client-secret");
    let printed = format!("{request:?}");
    assert!(printed.contains("[redacted]"));
    assert!(!printed.contains("client-secret"));
    assert!(!printed.contains("Y2xpZW50LWlkOmNsaWVudC1zZWNyZXQ="));
}

#[rstest]
#[case::path_and_query("https://host/base?x=1#f", "https://host/oidc/v1/token")]
#[case::port("https://host:8443/base", "https://host:8443/oidc/v1/token")]
fn the_token_request_drops_the_base_path(#[case] base: &str, #[case] expected: &str) {
    assert_eq!(oauth_token_request(base, "id", "secret").url, expected);
}

#[test]
fn a_base_without_a_scheme_keeps_only_the_token_path() {
    assert_eq!(
        oauth_token_request("host/base", "id", "secret").url,
        "/oidc/v1/token"
    );
}

#[test]
fn a_200_reply_yields_its_access_token() {
    let token = parse_oauth_token_reply(200, r#"{"access_token":"abc"}"#)
        .expect("the reply carries a token");
    assert_eq!(token.expose_secret(), "abc");
}

#[test]
fn a_reply_does_not_print_its_token() {
    let reply = OauthReply::Reply {
        status: 200,
        body: SecretString::from(r#"{"access_token":"abc"}"#),
    };
    let printed = format!("{reply:?}");
    assert!(printed.contains("REDACTED"));
    assert!(!printed.contains("access_token"));
    assert!(!printed.contains("abc"));
}

#[rstest]
#[case::missing_field("{}")]
#[case::not_json("not json")]
#[case::wrong_type(r#"{"access_token":5}"#)]
fn a_200_reply_without_a_token_is_a_typed_500(#[case] body: &str) {
    let error = parse_oauth_token_reply(200, body).expect_err("the reply has no token");
    assert_eq!(error.status_code, 500);
    assert!(error.message.contains("access_token"));
}

#[test]
fn a_non_200_reply_keeps_its_status_and_body() {
    let error = parse_oauth_token_reply(401, "no").expect_err("a 401 is a failure");
    assert_eq!(error.status_code, 401);
    assert_eq!(error.message, "OAuth M2M token request failed: no");
}

#[test]
fn a_transport_error_becomes_a_500() {
    let source = FixedSource(OauthReply::Transport("timed out".to_owned()));
    let request = oauth_token_request("https://host", "id", "secret");
    let runtime = Builder::new_current_thread()
        .build()
        .expect("the runtime builds");
    let outcome = runtime.block_on(request_oauth_token(&request, &source));
    let error = outcome.expect_err("a transport failure is an error");
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "OAuth M2M token request failed: timed out");
}

#[test]
fn a_failing_reply_keeps_its_own_status() {
    let source = FixedSource(OauthReply::Reply {
        status: 403,
        body: SecretString::from("denied"),
    });
    let request = oauth_token_request("https://host", "id", "secret");
    let runtime = Builder::new_current_thread()
        .build()
        .expect("the runtime builds");
    let outcome = runtime.block_on(request_oauth_token(&request, &source));
    let error = outcome.expect_err("a 403 is an error");
    assert_eq!(error.status_code, 403);
}

#[test]
fn replays_the_databricks_oauth_reply_corpus() {
    let seen = fs::read_dir("fuzz/corpus/databricks_oauth_reply")
        .expect("the corpus folder exists")
        .map(|entry| entry.expect("the entry reads").path())
        .filter(|path| path.is_file())
        .inspect(|path| {
            let bytes = fs::read(path).expect("the seed reads");
            let Some((&status, body)) = bytes.split_first() else {
                return;
            };
            let Ok(payload) = from_utf8(body) else {
                return;
            };
            let _token = parse_oauth_token_reply(u16::from(status), payload);
        })
        .count();
    assert!(seen > 0_usize);
}
