#![cfg(test)]

use crate::gitlab_client::GitLabClient;
use crate::gitlab_client::error::GitLabClientError;
use crate::gitlab_client::tests::{FakeTransport, config_from};

use super::GitLabPromptManager;

const CONFIG: &str = r#"{"project": "g/r", "access_token": "t", "prompts_path": "prompts"}"#;

#[test]
fn the_constructor_keeps_the_config_prompt_id_and_ref() {
    let manager = GitLabPromptManager::new(
        config_from(CONFIG),
        Some("chat/hi".to_owned()),
        Some("v2".to_owned()),
        None,
        FakeTransport::default(),
    )
    .unwrap();

    assert_eq!(manager.prompt_id.as_deref(), Some("chat/hi"));
    assert_eq!(manager.ref_override.as_deref(), Some("v2"));
    assert_eq!(
        manager.gitlab_config.prompts_path.as_deref(),
        Some("prompts")
    );
}

#[test]
fn integration_name_is_gitlab() {
    assert_eq!(
        GitLabPromptManager::<FakeTransport>::INTEGRATION_NAME,
        "gitlab"
    );
}

#[test]
fn an_injected_client_makes_an_empty_config_acceptable() {
    let client = GitLabClient::new(&config_from(CONFIG), FakeTransport::default()).unwrap();

    GitLabPromptManager::new(
        config_from("{}"),
        None,
        None,
        Some(client),
        FakeTransport::default(),
    )
    .unwrap();
}

#[test]
fn a_configured_prompt_id_ignores_the_injected_client() {
    let client = GitLabClient::new(&config_from(CONFIG), FakeTransport::default()).unwrap();

    let outcome = GitLabPromptManager::new(
        config_from("{}"),
        Some("chat/hi".to_owned()),
        None,
        Some(client),
        FakeTransport::default(),
    );

    assert!(matches!(
        outcome.err().unwrap(),
        GitLabClientError::MissingCredentials
    ));
}

#[test]
fn a_config_without_credentials_fails_at_construction() {
    let outcome = GitLabPromptManager::new(
        config_from(r#"{"project": "g/r"}"#),
        None,
        None,
        None,
        FakeTransport::default(),
    );

    assert!(matches!(
        outcome.err().unwrap(),
        GitLabClientError::MissingCredentials
    ));
}
