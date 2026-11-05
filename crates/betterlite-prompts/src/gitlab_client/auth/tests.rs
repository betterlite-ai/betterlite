#![cfg(test)]

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::request_headers;
use crate::gitlab_client::GitLabClient;
use crate::gitlab_client::error::GitLabClientError;
use crate::gitlab_client::tests::{FakeTransport, answer, config_from};

fn header_pairs(auth_method: &str) -> Vec<(String, String)> {
    request_headers(auth_method, &SecretString::from("tok-123"))
        .iter()
        .map(|header| {
            (
                header.name.to_owned(),
                header.value.expose_secret().to_owned(),
            )
        })
        .collect()
}

#[rstest]
#[case::token("token")]
#[case::empty("")]
#[case::unknown("basic")]
#[case::capitalised_oauth("OAuth")]
fn every_method_but_oauth_sends_private_token(#[case] auth_method: &str) {
    assert_eq!(
        header_pairs(auth_method),
        vec![
            ("Accept".to_owned(), "application/json".to_owned()),
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("Private-Token".to_owned(), "tok-123".to_owned()),
        ]
    );
}

#[test]
fn oauth_sends_a_bearer_authorization_header() {
    assert_eq!(
        header_pairs("oauth"),
        vec![
            ("Accept".to_owned(), "application/json".to_owned()),
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("Authorization".to_owned(), "Bearer tok-123".to_owned()),
        ]
    );
}

#[rstest]
#[case::default_is_token(
    r#"{"project": "g/r", "access_token": "tok-123"}"#,
    "Private-Token",
    "tok-123"
)]
#[case::null_is_token(
    r#"{"project": "g/r", "access_token": "tok-123", "auth_method": null}"#,
    "Private-Token",
    "tok-123"
)]
#[case::oauth(
    r#"{"project": "g/r", "access_token": "tok-123", "auth_method": "oauth"}"#,
    "Authorization",
    "Bearer tok-123"
)]
#[tokio::test]
async fn the_configured_method_picks_the_header_sent_with_each_request(
    #[case] json_text: &str,
    #[case] header: &str,
    #[case] value: &str,
) {
    let transport = FakeTransport::scripted(vec![answer!(200, "x")]);
    let client = GitLabClient::new(&config_from(json_text), transport.clone()).unwrap();
    client.get_file_content("a.prompt", None).await.unwrap();
    let sent = transport.requests();
    assert!(
        sent[0]
            .headers
            .contains(&(header.to_owned(), value.to_owned()))
    );
    assert_eq!(sent[0].headers.len(), 3);
}

#[test]
fn set_ref_rejects_an_empty_ref_and_keeps_the_old_one() {
    let mut client = GitLabClient::new(
        &config_from(r#"{"project": "g/r", "access_token": "t", "branch": "dev"}"#),
        FakeTransport::default(),
    )
    .unwrap();
    assert!(matches!(
        client.set_ref(""),
        Err(GitLabClientError::EmptyRef)
    ));
    assert_eq!(client.ref_name, "dev");
    assert_eq!(
        GitLabClientError::EmptyRef.to_string(),
        "ref must be a non-empty string"
    );
}

#[tokio::test]
async fn set_ref_changes_the_ref_of_later_requests() {
    let transport = FakeTransport::scripted(vec![answer!(200, "x")]);
    let mut client = GitLabClient::new(
        &config_from(r#"{"project": "g/r", "access_token": "t", "tag": "v1"}"#),
        transport.clone(),
    )
    .unwrap();
    client.set_ref("release/2").unwrap();
    client.get_file_content("a.prompt", None).await.unwrap();
    assert!(
        transport.requests()[0]
            .url
            .ends_with("/raw?ref=release%2F2")
    );
}
