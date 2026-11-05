#![cfg(test)]

use rstest::rstest;

use crate::gitlab_client::GitLabClient;
use crate::gitlab_client::error::GitLabClientError;
use crate::gitlab_client::tests::{FakeTransport, answer, config_from};

#[rstest]
#[case::no_project(r#"{"access_token": "t"}"#)]
#[case::no_token(r#"{"project": "g/r"}"#)]
#[case::null_project(r#"{"project": null, "access_token": "t"}"#)]
#[case::null_token(r#"{"project": "g/r", "access_token": null}"#)]
#[case::empty_project(r#"{"project": "", "access_token": "t"}"#)]
#[case::empty_token(r#"{"project": "g/r", "access_token": ""}"#)]
#[case::zero_project_id(r#"{"project": 0, "access_token": "t"}"#)]
#[case::empty_config("{}")]
fn missing_or_blank_credentials_are_refused(#[case] json_text: &str) {
    let outcome = GitLabClient::new(&config_from(json_text), FakeTransport::default());
    assert!(matches!(
        outcome,
        Err(GitLabClientError::MissingCredentials)
    ));
}

#[test]
fn the_missing_credentials_message_names_both_keys() {
    assert_eq!(
        GitLabClientError::MissingCredentials.to_string(),
        "project and access_token are required"
    );
}

#[rstest]
#[case::path(
    r#"{"project": "group/sub/repo", "access_token": "t"}"#,
    "group/sub/repo",
    "group%2Fsub%2Frepo"
)]
#[case::numeric(r#"{"project": 42, "access_token": "t"}"#, "42", "42")]
#[tokio::test]
async fn a_project_is_stored_and_url_encoded(
    #[case] json_text: &str,
    #[case] project: &str,
    #[case] encoded: &str,
) {
    let transport = FakeTransport::scripted(vec![answer(200, "x")]);
    let client = GitLabClient::new(&config_from(json_text), transport.clone()).unwrap();
    client.get_file_content("a", None).await.unwrap();
    assert_eq!(client.project, project);
    assert!(
        transport.requests()[0]
            .url
            .contains(&format!("/projects/{encoded}/"))
    );
}
