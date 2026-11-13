use std::task::Poll;

use aws_lc_rs::signature::ECDSA_P256_SHA256_FIXED;
use betterlite_config::sections::gdc_settings::BoolOrString;
use futures_util::future::join;
use futures_util::poll;
use secrecy::{ExposeSecret as _, SecretString};

use crate::gdc_auth::authenticator::credential_identity;
use crate::gdc_auth::cache::{CacheKey, CredentialIdentity};
use crate::gdc_auth::error::GdcAuthError;
use crate::gdc_auth::headers::Headers;
use crate::gdc_auth::transport::{GdcClock as _, TokenTrust};

use super::support::{
    Harness, P256_PKCS8, P256_SEC1, Reply, Request, TOKEN_URI, answer, descriptor_json,
    descriptor_with_ca, jwt_claims, jwt_header, token_reply, verify_ecdsa,
};

const BASE: &str = "https://gateway.example/v1/projects/base-project/locations/eu";
const PLAIN_KEY: &str = "sk-plain-token";

fn status(error: &GdcAuthError) -> u16 {
    error.class().metadata().status_code
}

fn header(headers: &Headers, name: &str) -> Option<String> {
    headers
        .get(name)
        .map(|value| value.expose_secret().to_owned())
}

fn service_account_request(base: &str) -> Request {
    let mut request = Request::new(&descriptor_json("credential-project", P256_SEC1), base);
    request.call.vertex_project = Some("call-project".to_owned());
    request
}

#[rustfmt::skip]
#[derive(serde::Deserialize)]
struct ExchangeBody {
    grant_type: String,
    audience: String,
    requested_token_type: String,
    subject_token: String,
    subject_token_type: String,
}

#[tokio::test]
async fn a_plain_bearer_key_fills_the_authorization_and_metadata_headers() {
    let harness = Harness::fixture(Vec::new());
    let headers = harness
        .auth
        .validate_environment(Request::new(PLAIN_KEY, BASE).validation())
        .await
        .expect("projected headers");
    assert_eq!(
        header(&headers, "Authorization").as_deref(),
        Some("Bearer sk-plain-token")
    );
    assert_eq!(
        header(&headers, "Content-Type").as_deref(),
        Some("application/json")
    );
    assert_eq!(
        header(&headers, "x-goog-user-project").as_deref(),
        Some("projects/base-project")
    );
    assert_eq!(harness.transport.count(), 0);
}

#[tokio::test]
async fn a_lowercase_authorization_header_does_not_suppress_the_bearer_token() {
    let harness = Harness::fixture(Vec::new());
    let mut request = Request::new(PLAIN_KEY, BASE);
    request.headers.insert(
        "authorization".to_owned(),
        SecretString::from("custom".to_owned()),
    );
    let headers = harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("projected headers");
    assert_eq!(header(&headers, "authorization").as_deref(), Some("custom"));
    assert_eq!(
        header(&headers, "Authorization").as_deref(),
        Some("Bearer sk-plain-token")
    );
}

#[tokio::test]
async fn an_exact_authorization_header_survives_a_plain_bearer_key() {
    let harness = Harness::fixture(Vec::new());
    let mut request = Request::new(PLAIN_KEY, BASE);
    request.headers.insert(
        "Authorization".to_owned(),
        SecretString::from("Bearer own".to_owned()),
    );
    let headers = harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("projected headers");
    assert_eq!(
        header(&headers, "Authorization").as_deref(),
        Some("Bearer own")
    );
}

#[rstest::rstest]
#[case::lower("content-type")]
#[case::exact("Content-Type")]
#[tokio::test]
async fn a_content_type_in_either_case_is_kept(#[case] name: &str) {
    let harness = Harness::fixture(Vec::new());
    let mut request = Request::new(PLAIN_KEY, BASE);
    request
        .headers
        .insert(name.to_owned(), SecretString::from("text/plain".to_owned()));
    let headers = harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("projected headers");
    assert_eq!(header(&headers, name).as_deref(), Some("text/plain"));
    assert_eq!(headers.len(), 3);
}

#[tokio::test]
async fn a_differently_cased_content_type_does_not_suppress_the_json_one() {
    let harness = Harness::fixture(Vec::new());
    let mut request = Request::new(PLAIN_KEY, BASE);
    request.headers.insert(
        "Content-type".to_owned(),
        SecretString::from("text/plain".to_owned()),
    );
    let headers = harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("projected headers");
    assert_eq!(
        header(&headers, "Content-Type").as_deref(),
        Some("application/json")
    );
}

#[tokio::test]
async fn every_case_variant_of_the_quota_project_header_is_replaced() {
    let harness = Harness::fixture(Vec::new());
    let mut request = Request::new(PLAIN_KEY, BASE);
    request.headers.insert(
        "x-goog-user-project".to_owned(),
        SecretString::from("projects/stale".to_owned()),
    );
    request.headers.insert(
        "X-Goog-User-Project".to_owned(),
        SecretString::from("projects/stale".to_owned()),
    );
    request.headers.insert(
        "X-GOOG-USER-PROJECT".to_owned(),
        SecretString::from("projects/stale".to_owned()),
    );
    let headers = harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("projected headers");
    let quota: Vec<(&str, &str)> = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("x-goog-user-project"))
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect();
    assert_eq!(
        quota,
        vec![("x-goog-user-project", "projects/base-project")]
    );
}

#[tokio::test]
async fn a_service_account_token_overwrites_the_exact_authorization_header() {
    let harness = Harness::pem(vec![answer(token_reply(Some("3600")))]);
    let mut request = service_account_request(BASE);
    request.headers.insert(
        "Authorization".to_owned(),
        SecretString::from("Bearer stale".to_owned()),
    );
    request.headers.insert(
        "authorization".to_owned(),
        SecretString::from("lowercase".to_owned()),
    );
    let headers = harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("projected headers");
    assert_eq!(
        header(&headers, "Authorization").as_deref(),
        Some("Bearer exchanged-token")
    );
    assert_eq!(
        header(&headers, "authorization").as_deref(),
        Some("lowercase")
    );
}

#[tokio::test]
async fn the_exchange_request_carries_the_audience_and_token_types() {
    let harness = Harness::pem(vec![answer(token_reply(Some("3600")))]);
    let request = service_account_request("https://gateway.example:8443/some/path?q=1#frag");
    let headers = harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("projected headers");
    assert_eq!(
        header(&headers, "Authorization").as_deref(),
        Some("Bearer exchanged-token")
    );
    assert_eq!(
        header(&headers, "x-goog-user-project").as_deref(),
        Some("projects/call-project")
    );
    let records = harness.transport.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].url, TOKEN_URI);
    let body: ExchangeBody = serde_json::from_str(&records[0].body).expect("a typed exchange body");
    assert_eq!(
        body.grant_type,
        "urn:ietf:params:oauth:token-type:token-exchange"
    );
    assert_eq!(
        body.requested_token_type,
        "urn:ietf:params:oauth:token-type:access_token"
    );
    assert_eq!(
        body.subject_token_type,
        "urn:k8s:params:oauth:token-type:serviceaccount"
    );
    assert_eq!(body.audience, "https://gateway.example:8443");
}

#[tokio::test]
async fn the_subject_token_is_a_signed_gdch_jwt() {
    let harness = Harness::pem(vec![answer(token_reply(Some("3600")))]);
    let request = service_account_request(BASE);
    harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("projected headers");
    let body: ExchangeBody =
        serde_json::from_str(&harness.transport.records()[0].body).expect("a typed exchange body");
    let token = body.subject_token;
    assert_eq!(jwt_header(&token).alg, "ES256");
    assert_eq!(jwt_header(&token).kid.as_deref(), Some("key-id"));
    let claims = jwt_claims(&token);
    assert_eq!(
        claims.iss,
        "system:serviceaccount:credential-project:service-identity"
    );
    assert_eq!(claims.sub, claims.iss);
    assert_eq!(claims.aud, TOKEN_URI);
    assert_eq!(claims.iat, harness.clock.unix_seconds());
    assert_eq!(claims.exp, claims.iat.saturating_add(3_600));
    assert!(verify_ecdsa(P256_PKCS8, &ECDSA_P256_SHA256_FIXED, &token));
}

#[tokio::test]
async fn the_credential_ca_certificate_overrides_ssl_verify() {
    let harness = Harness::pem(vec![answer(token_reply(Some("60")))]);
    let mut request = Request::new(
        &descriptor_with_ca("credential-project", P256_SEC1, "ca.pem"),
        BASE,
    );
    request.call.vertex_project = Some("call-project".to_owned());
    request.ssl_verify = BoolOrString::Bool(false);
    harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("projected headers");
    assert_eq!(harness.ca.paths(), vec!["ca.pem".to_owned()]);
    assert!(matches!(
        harness.transport.records()[0].trust,
        TokenTrust::Certificate(_)
    ));
}

#[tokio::test]
async fn a_ca_path_in_the_ssl_verify_setting_is_loaded() {
    let harness = Harness::pem(vec![answer(token_reply(Some("60")))]);
    let mut request = service_account_request(BASE);
    request.ssl_verify = BoolOrString::Text("bundle.pem".to_owned());
    harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("projected headers");
    assert_eq!(harness.ca.paths(), vec!["bundle.pem".to_owned()]);
}

#[tokio::test]
async fn ssl_verify_false_without_a_local_ca_means_insecure() {
    let harness = Harness::pem(vec![answer(token_reply(Some("60")))]);
    let mut request = service_account_request(BASE);
    request.ssl_verify = BoolOrString::Bool(false);
    harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("projected headers");
    assert!(matches!(
        harness.transport.records()[0].trust,
        TokenTrust::Insecure
    ));
}

#[tokio::test]
async fn the_cache_reuses_a_token_until_225_seconds_before_its_expiry() {
    let harness = Harness::pem(vec![
        answer(token_reply(Some("300"))),
        answer(token_reply(Some("300"))),
    ]);
    let mut request = service_account_request(BASE);
    request.caching = true;
    harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("first call");
    harness.clock.advance_seconds(74);
    harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("cached call");
    assert_eq!(harness.transport.count(), 1);
    harness.clock.advance_seconds(1);
    harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("refreshing call");
    assert_eq!(harness.transport.count(), 2);
}

#[tokio::test]
async fn a_token_without_an_expiry_is_reused_forever() {
    let harness = Harness::pem(vec![answer(token_reply(None)), answer(token_reply(None))]);
    let mut request = service_account_request(BASE);
    request.caching = true;
    harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("first call");
    harness.clock.advance_seconds(1_000_000);
    harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("cached call");
    assert_eq!(harness.transport.count(), 1);
}

#[tokio::test]
async fn a_failed_refresh_keeps_the_bound_handle_for_the_next_attempt() {
    let harness = Harness::pem(vec![Reply::Failure, answer(token_reply(Some("60")))]);
    let mut request = service_account_request(BASE);
    request.caching = true;
    let error = harness
        .auth
        .validate_environment(request.validation())
        .await
        .unwrap_err();
    assert_eq!(status(&error), 500);
    assert_eq!(harness.auth.cache.cache_keys().len(), 1);
    harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("the retry succeeds");
    assert_eq!(harness.transport.count(), 2);
}

#[tokio::test]
async fn cross_audience_calls_keep_separate_cache_entries() {
    let harness = Harness::pem(vec![
        answer(token_reply(Some("300"))),
        answer(token_reply(Some("300"))),
    ]);
    let mut first = service_account_request(BASE);
    first.caching = true;
    let mut second =
        service_account_request("https://other.example/v1/projects/base-project/locations/eu");
    second.caching = true;
    harness
        .auth
        .validate_environment(first.validation())
        .await
        .expect("first audience");
    harness
        .auth
        .validate_environment(second.validation())
        .await
        .expect("second audience");
    assert_eq!(harness.transport.count(), 2);
    assert_eq!(harness.auth.cache.cache_keys().len(), 2);
}

#[tokio::test]
async fn cross_credential_calls_keep_separate_cache_entries() {
    let harness = Harness::pem(vec![
        answer(token_reply(Some("300"))),
        answer(token_reply(Some("300"))),
    ]);
    let mut first = service_account_request(BASE);
    first.caching = true;
    let mut second = Request::new(&descriptor_json("other-project", P256_SEC1), BASE);
    second.call.vertex_project = Some("call-project".to_owned());
    second.caching = true;
    harness
        .auth
        .validate_environment(first.validation())
        .await
        .expect("first credential");
    harness
        .auth
        .validate_environment(second.validation())
        .await
        .expect("second credential");
    assert_eq!(harness.transport.count(), 2);
    assert_eq!(harness.auth.cache.cache_keys().len(), 2);
}

#[tokio::test]
async fn simultaneous_callers_share_one_refresh() {
    let harness = Harness::pem(vec![Reply::Yield, answer(token_reply(Some("300")))]);
    let mut request = service_account_request(BASE);
    request.caching = true;
    let first = harness.auth.validate_environment(request.validation());
    let second = harness.auth.validate_environment(request.validation());
    let (first_result, second_result) = join(first, second).await;
    let _first_headers = first_result.expect("the first caller gets a token");
    let _second_headers = second_result.expect("the second caller reuses the token");
    assert_eq!(harness.transport.count(), 1);
}

#[tokio::test]
async fn a_cancelled_refresh_releases_the_lease() {
    let harness = Harness::pem(vec![Reply::Pending, answer(token_reply(Some("300")))]);
    let mut request = service_account_request(BASE);
    request.caching = true;
    let mut cancelled = Box::pin(harness.auth.validate_environment(request.validation()));
    assert!(matches!(poll!(cancelled.as_mut()), Poll::Pending));
    drop(cancelled);
    let headers = harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("the lease is free");
    assert_eq!(
        header(&headers, "Authorization").as_deref(),
        Some("Bearer exchanged-token")
    );
    assert_eq!(harness.transport.count(), 2);
}

#[tokio::test]
async fn a_fixture_credential_never_opens_a_ca_path_or_uses_the_transport() {
    let harness = Harness::fixture(Vec::new());
    let mut request = Request::new(
        &descriptor_with_ca("credential-project", P256_SEC1, "ca.pem"),
        BASE,
    );
    request.call.vertex_project = Some("call-project".to_owned());
    request.ssl_verify = BoolOrString::Bool(false);
    request.caching = true;
    let headers = harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("projected headers");
    assert_eq!(
        header(&headers, "Authorization").as_deref(),
        Some("Bearer fixture-token")
    );
    assert!(harness.ca.paths().is_empty());
    assert_eq!(harness.transport.count(), 0);
    assert_eq!(
        harness.auth.cache.cache_keys(),
        vec![CacheKey {
            audience: "https://gateway.example".to_owned(),
            identity: CredentialIdentity::ApiKey(request.api_key.clone()),
        }]
    );
}

#[tokio::test]
async fn the_cache_identity_is_the_exact_key_or_the_injected_source_handle() {
    let key = SecretString::from("secret-key".to_owned());
    assert!(
        matches!(credential_identity(Some(&key), "handle"), CredentialIdentity::ApiKey(value) if value.expose_secret() == "secret-key")
    );
    assert!(
        matches!(credential_identity(None, "handle"), CredentialIdentity::SourceHandle(value) if value == "handle")
    );
}

#[tokio::test]
async fn missing_inputs_are_authentication_failures_in_litellm_order() {
    let harness = Harness::fixture(Vec::new());
    let mut no_base = Request::new(PLAIN_KEY, "");
    no_base.call.vertex_project = Some("call-project".to_owned());
    let base_error = harness
        .auth
        .validate_environment(no_base.validation())
        .await
        .unwrap_err();
    assert_eq!(status(&base_error), 401);
    assert!(base_error.to_string().contains("base URL"));

    let mut no_key = Request::new("", BASE);
    no_key.call.vertex_project = Some("call-project".to_owned());
    let key_error = harness
        .auth
        .validate_environment(no_key.validation())
        .await
        .unwrap_err();
    assert_eq!(status(&key_error), 401);
    assert!(key_error.to_string().contains("API key"));

    let project_error = harness
        .auth
        .validate_environment(Request::new(PLAIN_KEY, "https://gateway.example").validation())
        .await
        .unwrap_err();
    assert_eq!(status(&project_error), 401);
    assert!(project_error.to_string().contains("project"));
}

#[tokio::test]
async fn a_bad_project_is_an_authentication_failure_before_the_key_is_read() {
    let harness = Harness::fixture(Vec::new());
    let mut request = Request::new("not json at all", "https://gateway.example");
    request.call.vertex_project = Some("bad/project".to_owned());
    let error = harness
        .auth
        .validate_environment(request.validation())
        .await
        .unwrap_err();
    assert_eq!(status(&error), 401);
    assert!(error.to_string().contains("vertex_project"));
}

#[tokio::test]
async fn an_unparsable_base_for_the_audience_is_a_connection_failure() {
    let harness = Harness::fixture(Vec::new());
    let mut request = Request::new(PLAIN_KEY, "https://[fd00::1/x");
    request.call.vertex_project = Some("call-project".to_owned());
    let error = harness
        .auth
        .validate_environment(request.validation())
        .await
        .unwrap_err();
    assert_eq!(status(&error), 500);
}

#[tokio::test]
async fn a_non_gdch_json_credential_is_rejected_without_an_exchange() {
    let harness = Harness::pem(Vec::new());
    let mut request = Request::new("{\"type\":\"service_account\"}", BASE);
    request.call.vertex_project = Some("call-project".to_owned());
    let error = harness
        .auth
        .validate_environment(request.validation())
        .await
        .unwrap_err();
    assert_eq!(status(&error), 401);
    assert_eq!(harness.transport.count(), 0);
}

#[tokio::test]
async fn an_unloadable_private_key_is_rejected_without_an_exchange() {
    let harness = Harness::pem(Vec::new());
    let mut request = Request::new(&descriptor_json("credential-project", "not a pem"), BASE);
    request.call.vertex_project = Some("call-project".to_owned());
    let error = harness
        .auth
        .validate_environment(request.validation())
        .await
        .unwrap_err();
    assert_eq!(status(&error), 401);
    assert_eq!(harness.transport.count(), 0);
}

#[tokio::test]
async fn a_rejected_token_exchange_is_a_connection_failure() {
    let harness = Harness::pem(vec![Reply::Answer {
        status: 400,
        body: b"{\"error\":\"invalid_grant\"}".to_vec(),
    }]);
    let mut request = service_account_request(BASE);
    request.call.vertex_project = Some("call-project".to_owned());
    let error = harness
        .auth
        .validate_environment(request.validation())
        .await
        .unwrap_err();
    assert_eq!(status(&error), 500);
}

#[tokio::test]
async fn an_unusable_token_reply_is_a_connection_failure() {
    let harness = Harness::pem(vec![answer(b"{}".to_vec())]);
    let mut request = service_account_request(BASE);
    request.call.vertex_project = Some("call-project".to_owned());
    let error = harness
        .auth
        .validate_environment(request.validation())
        .await
        .unwrap_err();
    assert_eq!(status(&error), 500);
}

#[tokio::test]
async fn the_quota_project_keeps_a_terminal_newline_from_the_base() {
    let harness = Harness::fixture(Vec::new());
    let request = Request::new(
        PLAIN_KEY,
        "https://gateway.example/v1/projects/project-a\n/locations/eu",
    );
    let headers = harness
        .auth
        .validate_environment(request.validation())
        .await
        .expect("projected headers");
    assert_eq!(
        header(&headers, "x-goog-user-project").as_deref(),
        Some("projects/project-a\n")
    );
}
