#![cfg(test)]

use rstest::rstest;
use tokio::net::TcpListener;

use super::ClientError;
use crate::arize::phoenix::tests::server::{PROMPT_BODY, phoenix_client, serve};

#[tokio::test]
async fn client_fetches_the_version_with_bearer_authentication() {
    let phoenix = serve(200, PROMPT_BODY).await;
    let data = phoenix_client(&phoenix.base)
        .get_prompt_version("UHJvbXB0VmVyc2lvbjox")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(data.model_name.as_deref(), Some("gpt-4o"));
    let heads = phoenix.request_heads();
    assert_eq!(heads.len(), 1);
    assert!(heads[0].starts_with("GET /v1/prompt_versions/UHJvbXB0VmVyc2lvbjox HTTP/1.1\r\n"));
    assert!(heads[0].contains("authorization: Bearer secret-key\r\n"));
    assert!(heads[0].contains("accept: application/json\r\n"));
}

#[tokio::test]
async fn client_percent_encodes_the_identifier_in_the_path() {
    let phoenix = serve(200, PROMPT_BODY).await;
    phoenix_client(&phoenix.base)
        .get_prompt_version("abc==")
        .await
        .unwrap();
    assert!(phoenix.request_heads()[0].starts_with("GET /v1/prompt_versions/abc%3D%3D HTTP/1.1"));
}

#[rstest]
#[case::slash("a/b", "Invalid identifier 'a/b': contains disallowed characters")]
#[case::dot_dot("a..b", "Invalid identifier 'a..b': path traversal detected")]
#[tokio::test]
async fn client_error_rejects_an_unsafe_identifier_before_any_request(
    #[case] identifier: &str,
    #[case] message: &str,
) {
    let phoenix = serve(200, PROMPT_BODY).await;
    let error = phoenix_client(&phoenix.base)
        .get_prompt_version(identifier)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), message);
    assert!(phoenix.request_heads().is_empty());
}

#[rstest]
#[case::forbidden(
    403,
    "Access denied to prompt version 'v1'. Check your Arize Phoenix permissions."
)]
#[case::unauthorized(
    401,
    "Authentication failed. Check your Arize Phoenix API key and permissions."
)]
#[tokio::test]
async fn client_error_names_the_authentication_failure(#[case] status: u16, #[case] message: &str) {
    let phoenix = serve(status, "{}").await;
    let error = phoenix_client(&phoenix.base)
        .get_prompt_version("v1")
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), message);
}

#[tokio::test]
async fn client_error_403_and_401_are_different_errors() {
    let forbidden = serve(403, "{}").await;
    let unauthorized = serve(401, "{}").await;
    let denied = phoenix_client(&forbidden.base)
        .get_prompt_version("v1")
        .await;
    let rejected = phoenix_client(&unauthorized.base)
        .get_prompt_version("v1")
        .await;
    assert!(matches!(denied, Err(ClientError::AccessDenied { .. })));
    assert!(matches!(rejected, Err(ClientError::AuthenticationFailed)));
}

#[tokio::test]
async fn client_error_404_is_a_missing_version_not_a_failure() {
    let phoenix = serve(404, r#"{"detail":"not found"}"#).await;
    let found = phoenix_client(&phoenix.base)
        .get_prompt_version("v1")
        .await
        .unwrap();
    assert_eq!(found, None);
}

#[rstest]
#[case::server_error(500, "Server error '500 Internal Server Error'")]
#[case::bad_request(400, "Client error '400 Bad Request'")]
#[case::redirect(302, "Redirect response '302 Found'")]
#[tokio::test]
async fn client_error_other_statuses_carry_the_status_line(
    #[case] status: u16,
    #[case] kind: &str,
) {
    let phoenix = serve(status, "{}").await;
    let error = phoenix_client(&phoenix.base)
        .get_prompt_version("v1")
        .await
        .unwrap_err();
    let url = format!("{}/v1/prompt_versions/v1", phoenix.base);
    assert_eq!(
        error.to_string(),
        format!(
            "Failed to fetch prompt version 'v1': {kind} for url '{url}'\nFor more information check: https://developer.mozilla.org/en-US/docs/Web/HTTP/Status/{status}"
        )
    );
}

#[tokio::test]
async fn client_error_unreachable_server_is_an_error_fetching() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let error = phoenix_client(&base)
        .get_prompt_version("v1")
        .await
        .unwrap_err();
    assert!(matches!(error, ClientError::Transport { .. }));
    assert!(
        error
            .to_string()
            .starts_with("Error fetching prompt version 'v1': ")
    );
}

#[tokio::test]
async fn client_error_undecodable_body_is_an_error_fetching() {
    let phoenix = serve(200, "not json").await;
    let error = phoenix_client(&phoenix.base)
        .get_prompt_version("v1")
        .await
        .unwrap_err();
    assert!(matches!(error, ClientError::Decode { .. }));
    assert!(
        error
            .to_string()
            .starts_with("Error fetching prompt version 'v1': ")
    );
}

#[tokio::test]
async fn client_error_unparsable_base_is_an_error_fetching() {
    let error = phoenix_client("not a url")
        .get_prompt_version("v1")
        .await
        .unwrap_err();
    assert!(matches!(error, ClientError::InvalidUrl { .. }));
    assert!(
        error
            .to_string()
            .starts_with("Error fetching prompt version 'v1': ")
    );
}

#[tokio::test]
async fn client_answer_without_data_is_a_missing_version() {
    let phoenix = serve(200, r#"{"data":null}"#).await;
    let found = phoenix_client(&phoenix.base)
        .get_prompt_version("v1")
        .await
        .unwrap();
    assert_eq!(found, None);
}
