use betterlite_error::proxy_error_types::ProxyErrorTypes;
use rstest::rstest;

use crate::oidc_token_exchange::decode::ProviderClaims;
use crate::oidc_token_exchange::exchange::{OidcTokenExchangeError, exchange_pkce_code};

use super::support::{LocalProvider, Reply, base_request, client, text};

async fn exchange_against(
    token_body: &str,
    userinfo_body: &str,
) -> Result<ProviderClaims, OidcTokenExchangeError> {
    let provider = LocalProvider::start().await;
    provider.reply("/token", Reply::json(token_body));
    provider.reply("/userinfo", Reply::json(userinfo_body));
    exchange_pkce_code(&client(), &base_request(&provider)).await
}

#[tokio::test]
async fn keeps_the_token_response_bearer_fields_over_userinfo() {
    let claims = exchange_against(
        r#"{"access_token":"token-access","id_token":"token-id","refresh_token":"token-refresh","token_type":"Bearer"}"#,
        r#"{"sub":"user-1","access_token":"userinfo-access","id_token":"userinfo-id","refresh_token":"userinfo-refresh"}"#,
    )
    .await
    .unwrap();
    assert_eq!(text(&claims, "access_token"), Some("token-access"));
    assert_eq!(text(&claims, "id_token"), Some("token-id"));
    assert_eq!(text(&claims, "refresh_token"), Some("token-refresh"));
    assert_eq!(text(&claims, "token_type"), Some("Bearer"));
    assert_eq!(text(&claims, "sub"), Some("user-1"));
}

#[tokio::test]
async fn removes_an_explicitly_null_bearer_field_from_the_identity() {
    let claims = exchange_against(
        r#"{"access_token":"token-access","id_token":null}"#,
        r#"{"sub":"user-1","id_token":"userinfo-id","refresh_token":"userinfo-refresh"}"#,
    )
    .await
    .unwrap();
    assert!(!claims.contains_key("id_token"));
    assert_eq!(text(&claims, "sub"), Some("user-1"));
    assert_eq!(text(&claims, "refresh_token"), Some("userinfo-refresh"));
}

#[tokio::test]
async fn keeps_a_userinfo_bearer_field_the_token_response_omits() {
    let claims = exchange_against(
        r#"{"access_token":"token-access"}"#,
        r#"{"sub":"user-1","refresh_token":"userinfo-refresh"}"#,
    )
    .await
    .unwrap();
    assert_eq!(text(&claims, "refresh_token"), Some("userinfo-refresh"));
    assert!(!claims.contains_key("id_token"));
}

#[tokio::test]
async fn lets_userinfo_win_for_identity_claims() {
    let claims = exchange_against(
        r#"{"access_token":"token-access","sub":"token-sub","email":"token@example.com"}"#,
        r#"{"sub":"userinfo-sub","email":"userinfo@example.com","name":"Ada"}"#,
    )
    .await
    .unwrap();
    assert_eq!(text(&claims, "sub"), Some("userinfo-sub"));
    assert_eq!(text(&claims, "email"), Some("userinfo@example.com"));
    assert_eq!(text(&claims, "name"), Some("Ada"));
    assert_eq!(text(&claims, "access_token"), Some("token-access"));
}

#[rstest]
#[case::list("[1, 2]")]
#[case::string(r#""user-1""#)]
#[case::number("5")]
#[case::truthy("true")]
#[tokio::test]
async fn fails_with_a_500_when_userinfo_is_a_truthy_non_object(#[case] userinfo_body: &str) {
    let error = exchange_against(
        r#"{"access_token":"token-access","id_token":"eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ1c2VyLTEifQ."}"#,
        userinfo_body,
    )
    .await
    .unwrap_err();
    let response = error.response();
    assert_eq!(response.status, 500);
    assert_eq!(response.error_type, ProxyErrorTypes::InternalServerError);
    assert_eq!(response.param, None);
    assert_eq!(response.code, "500");
}

#[tokio::test]
async fn an_identity_with_only_userinfo_claims_still_carries_the_access_token() {
    let claims = exchange_against(r#"{"access_token":"token-access"}"#, r#"{"sub":"user-1"}"#)
        .await
        .unwrap();
    assert_eq!(text(&claims, "access_token"), Some("token-access"));
    assert_eq!(text(&claims, "sub"), Some("user-1"));
}
