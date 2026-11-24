#![cfg(test)]

use rstest::rstest;

use crate::gateway_dcr::oauth_error::{OauthError, OauthErrorCode, OauthErrorResponse as _};
use crate::oauth_resource::challenge::TOKEN_NO_CACHE_HEADERS;

fn serialized(code: OauthErrorCode) -> String {
    serde_json::to_string(&code).unwrap()
}

fn error() -> OauthError {
    OauthError {
        status: 400,
        error: OauthErrorCode::InvalidRedirectUri,
        description: "each redirect URI must be a valid URL".to_owned(),
    }
}

#[rstest]
#[case::invalid_request(OauthErrorCode::InvalidRequest, "invalid_request")]
#[case::invalid_client(OauthErrorCode::InvalidClient, "invalid_client")]
#[case::invalid_grant(OauthErrorCode::InvalidGrant, "invalid_grant")]
#[case::invalid_target(OauthErrorCode::InvalidTarget, "invalid_target")]
#[case::invalid_redirect_uri(OauthErrorCode::InvalidRedirectUri, "invalid_redirect_uri")]
#[case::invalid_client_metadata(OauthErrorCode::InvalidClientMetadata, "invalid_client_metadata")]
#[case::unsupported_response_type(
    OauthErrorCode::UnsupportedResponseType,
    "unsupported_response_type"
)]
#[case::unsupported_grant_type(OauthErrorCode::UnsupportedGrantType, "unsupported_grant_type")]
#[case::access_denied(OauthErrorCode::AccessDenied, "access_denied")]
#[case::login_required(OauthErrorCode::LoginRequired, "login_required")]
#[case::temporarily_unavailable(OauthErrorCode::TemporarilyUnavailable, "temporarily_unavailable")]
#[case::server_error(OauthErrorCode::ServerError, "server_error")]
fn every_code_renders_its_snake_case_text(#[case] code: OauthErrorCode, #[case] text: &str) {
    assert_eq!(<&'static str>::from(code), text);
    assert_eq!(serialized(code), format!("\"{text}\""));
    assert_eq!(code.to_string(), text);
}

#[test]
fn body_serializes_the_code_and_description() {
    assert_eq!(
        serde_json::to_string(&error().body()).unwrap(),
        r#"{"error":"invalid_redirect_uri","error_description":"each redirect URI must be a valid URL"}"#
    );
}

#[test]
fn every_error_carries_the_no_cache_headers() {
    let headers = error().headers();
    assert_eq!(headers, TOKEN_NO_CACHE_HEADERS);
    assert_eq!(
        headers,
        [("Cache-Control", "no-store"), ("Pragma", "no-cache")]
    );
}
