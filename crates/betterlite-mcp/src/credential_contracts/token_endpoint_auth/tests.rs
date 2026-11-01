#![cfg(test)]

use rstest::rstest;

use super::{TokenEndpointAuthMethod, build_token_client_auth, normalize_token_auth_method};

#[rstest]
fn normalize_token_auth_method_basic() {
    assert_eq!(
        normalize_token_auth_method("client_secret_basic"),
        Some(TokenEndpointAuthMethod::ClientSecretBasic)
    );
}

#[rstest]
fn normalize_token_auth_method_post() {
    assert_eq!(
        normalize_token_auth_method("client_secret_post"),
        Some(TokenEndpointAuthMethod::ClientSecretPost)
    );
}

#[rstest]
#[case::empty("")]
#[case::unknown("client_secret_jwt")]
#[case::wrong_case("Client_Secret_Basic")]
fn normalize_token_auth_method_unknown(#[case] raw: &str) {
    assert_eq!(normalize_token_auth_method(raw), None);
}

#[rstest]
fn build_token_client_auth_basic_quote_plus_encoding() {
    let auth = build_token_client_auth(
        Some(TokenEndpointAuthMethod::ClientSecretBasic),
        Some("client id+1"),
        Some("s3cr3t:val/ue"),
    )
    .expect("both credentials are present");
    assert!(auth.body.is_empty());
    assert_eq!(
        auth.headers.get("Authorization").map(String::as_str),
        Some("Basic Y2xpZW50K2lkJTJCMTpzM2NyM3QlM0F2YWwlMkZ1ZQ==")
    );
}

#[rstest]
fn build_token_client_auth_post_body() {
    let auth = build_token_client_auth(
        Some(TokenEndpointAuthMethod::ClientSecretPost),
        Some("my-client"),
        Some("my-secret"),
    )
    .expect("post never fails on present credentials");
    assert!(auth.headers.is_empty());
    assert_eq!(
        auth.body.get("client_id").map(String::as_str),
        Some("my-client")
    );
    assert_eq!(
        auth.body.get("client_secret").map(String::as_str),
        Some("my-secret")
    );
}

#[rstest]
#[case::missing_secret(Some("my-client"), None, true, false)]
#[case::missing_client_id(None, Some("my-secret"), false, true)]
fn build_token_client_auth_post_partial_credentials(
    #[case] client_id: Option<&str>,
    #[case] client_secret: Option<&str>,
    #[case] expect_client_id: bool,
    #[case] expect_client_secret: bool,
) {
    let auth = build_token_client_auth(None, client_id, client_secret)
        .expect("post never fails on a missing credential");
    assert_eq!(auth.body.contains_key("client_id"), expect_client_id);
    assert_eq!(
        auth.body.contains_key("client_secret"),
        expect_client_secret
    );
}

#[rstest]
fn build_token_client_auth_post_empty() {
    let auth = build_token_client_auth(None, None, None)
        .expect("post never fails when both credentials are absent");
    assert!(auth.headers.is_empty());
    assert!(auth.body.is_empty());
}

#[rstest]
#[case::missing_secret(Some("my-client"), None)]
#[case::missing_client_id(None, Some("my-secret"))]
#[case::both_missing(None, None)]
#[case::empty_secret(Some("my-client"), Some(""))]
fn build_token_client_auth_missing_secret_raises(
    #[case] client_id: Option<&str>,
    #[case] client_secret: Option<&str>,
) {
    let error = build_token_client_auth(
        Some(TokenEndpointAuthMethod::ClientSecretBasic),
        client_id,
        client_secret,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "token_endpoint_auth_method=client_secret_basic requires both client_id and client_secret"
    );
}
