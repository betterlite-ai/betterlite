#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;
use reqwest::blocking::ClientBuilder as BlockingClientBuilder;

use super::AgentRequestError;
use super::create_async::async_create_agent;
use super::create_sync::create_agent;
use super::get_async::async_get_agent;
use super::get_sync::get_agent;
use super::list_async::async_list_agents;
use super::list_sync::list_agents;
use super::support::{self, Config, Recorder, clients, params_at};
use crate::interactions::base::ClientError;
use crate::interactions::base::tests::support as base;
use crate::proxy::base_builder;

#[test]
fn client_create_resolves_a_blocking_client() {
    let serving = support::serve_post(base::reply("200 OK", &[], "{}"), None);
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let mut clients = clients();
    let mut logging = Recorder::default();

    create_agent(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::create_request("agent-1", &params),
    )
    .unwrap();

    assert_eq!(clients.sync_clients.len(), 1);
    assert!(clients.sync_clients.contains_key(&None));
    serving.request.join().unwrap();
}

#[test]
fn client_create_uses_a_supplied_blocking_client() {
    let serving = support::serve_post(base::reply("200 OK", &[], "{}"), None);
    let mut config = Config::for_address(serving.address);
    config.create_url = format!(
        "http://agent.invalid:{}/v1beta/agents",
        serving.address.port()
    );
    let params = params_at(serving.address);
    let supplied = BlockingClientBuilder::new()
        .resolve("agent.invalid", serving.address)
        .build()
        .unwrap();
    let mut clients = clients();
    let mut logging = Recorder::default();

    create_agent(
        &config,
        &mut clients,
        &mut logging,
        Some(supplied),
        &support::create_request("agent-1", &params),
    )
    .unwrap();

    assert!(clients.sync_clients.is_empty());
    serving.request.join().unwrap();
}

#[tokio::test]
async fn client_create_resolves_an_async_client() {
    let serving = support::serve_post(base::reply("200 OK", &[], "{}"), None);
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let mut clients = clients();
    let mut logging = Recorder::default();

    async_create_agent(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::create_request("agent-1", &params),
    )
    .await
    .unwrap();

    let providers: Vec<ProviderId> = clients
        .async_clients
        .keys()
        .map(|key| key.provider)
        .collect();
    assert_eq!(providers, vec![ProviderId::Gemini]);
    serving.request.join().unwrap();
}

#[tokio::test]
async fn client_create_uses_a_supplied_async_client() {
    let serving = support::serve_post(base::reply("200 OK", &[], "{}"), None);
    let mut config = Config::for_address(serving.address);
    config.create_url = format!(
        "http://agent.invalid:{}/v1beta/agents",
        serving.address.port()
    );
    let params = params_at(serving.address);
    let supplied = base_builder()
        .resolve("agent.invalid", serving.address)
        .build()
        .unwrap();
    let mut clients = clients();
    let mut logging = Recorder::default();

    async_create_agent(
        &config,
        &mut clients,
        &mut logging,
        Some(supplied),
        &support::create_request("agent-1", &params),
    )
    .await
    .unwrap();

    assert!(clients.async_clients.is_empty());
    serving.request.join().unwrap();
}

#[test]
fn client_list_resolves_a_blocking_client() {
    let serving = base::serve_once(base::reply("200 OK", &[], "{}"), None);
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let mut clients = clients();
    let mut logging = Recorder::default();

    list_agents(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::list_request(&params),
    )
    .unwrap();

    assert_eq!(clients.sync_clients.len(), 1);
    serving.request.join().unwrap();
}

#[test]
fn client_list_uses_a_supplied_blocking_client() {
    let serving = base::serve_once(base::reply("200 OK", &[], "{}"), None);
    let mut config = Config::for_address(serving.address);
    config.list.url = format!(
        "http://agent.invalid:{}/v1beta/agents",
        serving.address.port()
    );
    let params = params_at(serving.address);
    let supplied = BlockingClientBuilder::new()
        .resolve("agent.invalid", serving.address)
        .build()
        .unwrap();
    let mut clients = clients();
    let mut logging = Recorder::default();

    list_agents(
        &config,
        &mut clients,
        &mut logging,
        Some(supplied),
        &support::list_request(&params),
    )
    .unwrap();

    assert!(clients.sync_clients.is_empty());
    serving.request.join().unwrap();
}

#[tokio::test]
async fn client_list_resolves_an_async_client() {
    let serving = base::serve_once(base::reply("200 OK", &[], "{}"), None);
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let mut clients = clients();
    let mut logging = Recorder::default();

    async_list_agents(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::list_request(&params),
    )
    .await
    .unwrap();

    assert_eq!(clients.async_clients.len(), 1);
    serving.request.join().unwrap();
}

#[tokio::test]
async fn client_list_builds_the_async_client_for_the_named_provider() {
    let serving = base::serve_once(base::reply("200 OK", &[], "{}"), None);
    let config = Config::for_address(serving.address);
    let params = base::params(&format!(
        r#"{{"api_base": "http://{}", "custom_llm_provider": "openai"}}"#,
        serving.address
    ));
    let mut clients = clients();
    let mut logging = Recorder::default();

    async_list_agents(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::list_request(&params),
    )
    .await
    .unwrap();

    let providers: Vec<ProviderId> = clients
        .async_clients
        .keys()
        .map(|key| key.provider)
        .collect();
    assert_eq!(providers, vec![ProviderId::Openai]);
    serving.request.join().unwrap();
}

#[tokio::test]
async fn client_list_uses_a_supplied_async_client() {
    let serving = base::serve_once(base::reply("200 OK", &[], "{}"), None);
    let mut config = Config::for_address(serving.address);
    config.list.url = format!(
        "http://agent.invalid:{}/v1beta/agents",
        serving.address.port()
    );
    let params = params_at(serving.address);
    let supplied = base_builder()
        .resolve("agent.invalid", serving.address)
        .build()
        .unwrap();
    let mut clients = clients();
    let mut logging = Recorder::default();

    async_list_agents(
        &config,
        &mut clients,
        &mut logging,
        Some(supplied),
        &support::list_request(&params),
    )
    .await
    .unwrap();

    assert!(clients.async_clients.is_empty());
    serving.request.join().unwrap();
}

#[test]
fn client_get_resolves_a_blocking_client() {
    let serving = base::serve_once(base::reply("200 OK", &[], "{}"), None);
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let mut clients = clients();
    let mut logging = Recorder::default();

    get_agent(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::get_request("agent-1", &params),
    )
    .unwrap();

    assert_eq!(clients.sync_clients.len(), 1);
    serving.request.join().unwrap();
}

#[test]
fn client_get_uses_a_supplied_blocking_client() {
    let serving = base::serve_once(base::reply("200 OK", &[], "{}"), None);
    let mut config = Config::for_address(serving.address);
    config.item.url = format!(
        "http://agent.invalid:{}/v1beta/agents/agent-1",
        serving.address.port()
    );
    let params = params_at(serving.address);
    let supplied = BlockingClientBuilder::new()
        .resolve("agent.invalid", serving.address)
        .build()
        .unwrap();
    let mut clients = clients();
    let mut logging = Recorder::default();

    get_agent(
        &config,
        &mut clients,
        &mut logging,
        Some(supplied),
        &support::get_request("agent-1", &params),
    )
    .unwrap();

    assert!(clients.sync_clients.is_empty());
    serving.request.join().unwrap();
}

#[tokio::test]
async fn client_get_resolves_an_async_client() {
    let serving = base::serve_once(base::reply("200 OK", &[], "{}"), None);
    let config = Config::for_address(serving.address);
    let params = params_at(serving.address);
    let mut clients = clients();
    let mut logging = Recorder::default();

    async_get_agent(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::get_request("agent-1", &params),
    )
    .await
    .unwrap();

    assert_eq!(clients.async_clients.len(), 1);
    serving.request.join().unwrap();
}

#[tokio::test]
async fn client_get_uses_a_supplied_async_client() {
    let serving = base::serve_once(base::reply("200 OK", &[], "{}"), None);
    let mut config = Config::for_address(serving.address);
    config.item.url = format!(
        "http://agent.invalid:{}/v1beta/agents/agent-1",
        serving.address.port()
    );
    let params = params_at(serving.address);
    let supplied = base_builder()
        .resolve("agent.invalid", serving.address)
        .build()
        .unwrap();
    let mut clients = clients();
    let mut logging = Recorder::default();

    async_get_agent(
        &config,
        &mut clients,
        &mut logging,
        Some(supplied),
        &support::get_request("agent-1", &params),
    )
    .await
    .unwrap();

    assert!(clients.async_clients.is_empty());
    serving.request.join().unwrap();
}

#[tokio::test]
async fn client_create_reports_an_unknown_async_provider() {
    let config = Config::for_address(support::dead_address());
    let params = base::params(r#"{"custom_llm_provider": "nope"}"#);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let error = async_create_agent(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::create_request("agent-1", &params),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(&error, AgentRequestError::Client(ClientError::UnknownProvider { provider }) if provider.as_str() == "nope"),
        "{error:?}"
    );
    assert!(config.calls().is_empty());
    assert!(logging.calls().is_empty());
}

#[test]
fn client_create_reports_a_non_boolean_ssl_verify() {
    let config = Config::for_address(support::dead_address());
    let params = base::params(r#"{"ssl_verify": "yes"}"#);
    let mut clients = clients();
    let mut logging = Recorder::default();

    let error = create_agent(
        &config,
        &mut clients,
        &mut logging,
        None,
        &support::create_request("agent-1", &params),
    )
    .unwrap_err();

    assert!(
        matches!(
            &error,
            AgentRequestError::Client(ClientError::FieldType {
                field: "ssl_verify",
                ..
            })
        ),
        "{error:?}"
    );
    assert!(config.calls().is_empty());
}
