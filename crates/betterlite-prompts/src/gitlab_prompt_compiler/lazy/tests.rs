#![cfg(test)]

use std::io;

use crate::gitlab_client::GitLabClient;
use crate::gitlab_client::tests::{FakeTransport, answer, config_from};
use crate::gitlab_client::transport::GitLabResponse;

use super::super::init::manager::GitLabPromptManager;

const CONFIG: &str = r#"{"project": "g/r", "access_token": "t", "prompts_path": "prompts"}"#;

type Scripted = Vec<Result<GitLabResponse, io::Error>>;

async fn manager(
    config_json: &str,
    prompt_id: Option<&str>,
    responses: Scripted,
) -> (GitLabPromptManager<FakeTransport>, FakeTransport) {
    let transport = FakeTransport::scripted(responses);
    let manager = GitLabPromptManager::new(
        config_from(config_json),
        prompt_id.map(str::to_owned),
        None,
        None,
        transport.clone(),
    )
    .await
    .expect("the config builds a client");
    (manager, transport)
}

#[tokio::test]
async fn the_configured_prompt_is_fetched_on_the_first_access() {
    let (manager, transport) = manager(CONFIG, Some("greet"), vec![answer!(200, "User: hi")]).await;

    assert!(transport.requests().is_empty());

    let loaded = {
        let slot = manager.prompt_manager().await.expect("the manager loads");
        slot.manager.prompts.contains_key("greet")
    };

    assert!(loaded);
    assert_eq!(transport.requests().len(), 1);
}

#[tokio::test]
async fn the_configured_prompt_is_loaded_only_once() {
    let (manager, transport) = manager(CONFIG, Some("greet"), vec![answer!(200, "User: hi")]).await;

    drop(manager.prompt_manager().await.expect("the manager loads"));
    drop(manager.prompt_manager().await.expect("the manager loads"));

    assert_eq!(transport.requests().len(), 1);
}

#[tokio::test]
async fn a_manager_without_a_prompt_id_fetches_nothing() {
    let (manager, transport) = manager(CONFIG, None, vec![]).await;

    let slot = manager.prompt_manager().await.expect("the manager loads");

    assert!(slot.configured_loaded);
    drop(slot);
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn an_empty_prompt_id_is_not_configured_and_keeps_the_injected_client() {
    let injected_transport = FakeTransport::scripted(vec![answer!(200, "User: hi")]);
    let client = GitLabClient::new(
        &config_from(r#"{"project": "own/project", "access_token": "t", "tag": "v9"}"#),
        injected_transport.clone(),
    )
    .unwrap();
    let transport = FakeTransport::scripted(vec![]);
    let manager = GitLabPromptManager::new(
        config_from("{}"),
        Some(String::new()),
        None,
        Some(client),
        transport.clone(),
    )
    .await
    .unwrap();

    let (rendered, _metadata) = manager
        .get_prompt_template("chat/hi", None, None)
        .await
        .unwrap();

    assert_eq!(rendered, "User: hi");
    assert_eq!(injected_transport.requests().len(), 1);
    assert!(transport.requests().is_empty());
    let url = &injected_transport.requests()[0].url;
    assert!(url.contains("/projects/own%2Fproject/"), "{url}");
}

#[tokio::test]
async fn an_empty_prompt_id_loads_nothing_up_front() {
    let (manager, transport) = manager(CONFIG, Some(""), vec![]).await;

    let loaded = {
        let slot = manager.prompt_manager().await.expect("the manager loads");
        slot.configured_loaded
    };

    assert!(loaded);
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn a_manager_uses_the_injected_client_when_no_prompt_id_is_configured() {
    let injected_transport = FakeTransport::scripted(vec![answer!(200, "User: hi")]);
    let client = GitLabClient::new(
        &config_from(r#"{"project": "own/project", "access_token": "t", "tag": "v9"}"#),
        injected_transport.clone(),
    )
    .unwrap();
    let transport = FakeTransport::scripted(vec![]);
    let manager = GitLabPromptManager::new(
        config_from("{}"),
        None,
        None,
        Some(client),
        transport.clone(),
    )
    .await
    .unwrap();

    let (rendered, _metadata) = manager
        .get_prompt_template("chat/hi", None, None)
        .await
        .unwrap();

    assert_eq!(rendered, "User: hi");
    assert_eq!(injected_transport.requests().len(), 1);
    assert!(transport.requests().is_empty());
    let url = &injected_transport.requests()[0].url;
    assert!(url.contains("/projects/own%2Fproject/"), "{url}");
    assert!(url.ends_with("?ref=v9"), "{url}");
}

#[tokio::test]
async fn a_configured_prompt_id_drops_the_injected_client() {
    let injected_transport = FakeTransport::scripted(vec![]);
    let client = GitLabClient::new(
        &config_from(r#"{"project": "own/project", "access_token": "t"}"#),
        injected_transport.clone(),
    )
    .unwrap();
    let transport = FakeTransport::scripted(vec![answer!(200, "User: hi")]);
    let manager = GitLabPromptManager::new(
        config_from(CONFIG),
        Some("greet".to_owned()),
        None,
        Some(client),
        transport.clone(),
    )
    .await
    .unwrap();

    drop(manager.prompt_manager().await.expect("the manager loads"));

    assert!(injected_transport.requests().is_empty());
    assert_eq!(transport.requests().len(), 1);
    assert!(transport.requests()[0].url.contains("/projects/g%2Fr/"));
}

#[tokio::test]
async fn a_failed_configured_load_is_retried_on_the_next_access() {
    let (manager, transport) = manager(
        CONFIG,
        Some("greet"),
        vec![
            Err(io::Error::other("unreachable")),
            answer!(200, "User: hi"),
        ],
    )
    .await;

    assert!(manager.prompt_manager().await.is_err());
    drop(manager.prompt_manager().await.expect("the retry succeeds"));

    assert_eq!(transport.requests().len(), 2);
}
