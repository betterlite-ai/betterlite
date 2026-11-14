use std::net::SocketAddr;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::identity::provider_id::ProviderId;
use reqwest::blocking::ClientBuilder as BlockingClientBuilder;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use super::AgentsDeleteVersionsError;
use super::asynchronous::{async_delete_agent, async_list_agent_versions};
use super::support::{Call, Fixture, clients, dead_address};
use super::synchronous::{delete_agent, list_agent_versions};
use crate::interactions::base::ClientError;
use crate::interactions::base::Logging;
use crate::interactions::base::tests::support as server;
use crate::proxy::base_builder;

fn marker_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-supplied"),
        HeaderValue::from_static("yes"),
    );
    headers
}

fn call_with_unusable_ssl_verify(address: SocketAddr) -> Call {
    let mut call = Call::new(address);
    call.params
        .insert("ssl_verify".to_owned(), JsonValue::Text("yes".to_owned()));
    call
}

#[test]
fn client_resolves_the_sync_client_through_the_base_module() {
    let fixture = Fixture::default();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = call_with_unusable_ssl_verify(dead_address());

    let deleted = delete_agent(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .unwrap_err();
    let listed = list_agent_versions(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .unwrap_err();

    assert!(
        matches!(
            deleted,
            AgentsDeleteVersionsError::Client(ClientError::FieldType {
                field: "ssl_verify",
                ..
            })
        ),
        "{deleted:?}"
    );
    assert!(
        matches!(
            listed,
            AgentsDeleteVersionsError::Client(ClientError::FieldType {
                field: "ssl_verify",
                ..
            })
        ),
        "{listed:?}"
    );
    assert!(clients.sync_clients.is_empty());
}

#[tokio::test]
async fn client_resolves_the_async_client_through_the_base_module() {
    let fixture = Fixture::default();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = call_with_unusable_ssl_verify(dead_address());

    let deleted = async_delete_agent(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .await
    .unwrap_err();
    let listed = async_list_agent_versions(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(
            deleted,
            AgentsDeleteVersionsError::Client(ClientError::FieldType {
                field: "ssl_verify",
                ..
            })
        ),
        "{deleted:?}"
    );
    assert!(
        matches!(
            listed,
            AgentsDeleteVersionsError::Client(ClientError::FieldType {
                field: "ssl_verify",
                ..
            })
        ),
        "{listed:?}"
    );
    assert!(clients.async_clients.is_empty());
}

#[tokio::test]
async fn client_resolves_the_async_client_for_the_provider_its_params_name() {
    let serving = server::serve_once(server::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::default();
    let mut clients = clients();
    let mut logging = Logging::default();
    let mut call = Call::new(serving.address);
    call.params.insert(
        "custom_llm_provider".to_owned(),
        JsonValue::Text("openai".to_owned()),
    );

    async_delete_agent(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &call.agent_request(),
    )
    .await
    .unwrap();

    let providers: Vec<ProviderId> = clients
        .async_clients
        .keys()
        .map(|key| key.provider)
        .collect();
    assert!(providers.contains(&ProviderId::Openai));
    assert!(!providers.contains(&ProviderId::Gemini));
    serving.request.join().unwrap();
}

#[test]
fn client_uses_the_supplied_sync_client_for_delete() {
    let serving = server::serve_once(server::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::default();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = call_with_unusable_ssl_verify(serving.address);
    let supplied = BlockingClientBuilder::new()
        .default_headers(marker_headers())
        .build()
        .unwrap();

    let output = delete_agent(
        &fixture,
        &mut clients,
        &mut logging,
        Some(supplied),
        &call.agent_request(),
    )
    .unwrap();

    assert_eq!(output, "delete|agent-1|200|2");
    let sent = serving.request.join().unwrap();
    assert!(sent.contains("x-supplied: yes\r\n"), "{sent}");
    assert!(clients.sync_clients.is_empty());
}

#[test]
fn client_uses_the_supplied_sync_client_for_versions() {
    let serving = server::serve_once(server::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::default();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = call_with_unusable_ssl_verify(serving.address);
    let supplied = BlockingClientBuilder::new()
        .default_headers(marker_headers())
        .build()
        .unwrap();

    let output = list_agent_versions(
        &fixture,
        &mut clients,
        &mut logging,
        Some(supplied),
        &call.agent_request(),
    )
    .unwrap();

    assert_eq!(output, "versions|agent-1|200|2");
    let sent = serving.request.join().unwrap();
    assert!(sent.contains("x-supplied: yes\r\n"), "{sent}");
    assert!(clients.sync_clients.is_empty());
}

#[tokio::test]
async fn client_uses_the_supplied_async_client_for_delete() {
    let serving = server::serve_once(server::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::default();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = call_with_unusable_ssl_verify(serving.address);
    let supplied = base_builder()
        .default_headers(marker_headers())
        .build()
        .unwrap();

    let output = async_delete_agent(
        &fixture,
        &mut clients,
        &mut logging,
        Some(supplied),
        &call.agent_request(),
    )
    .await
    .unwrap();

    assert_eq!(output, "delete|agent-1|200|2");
    let sent = serving.request.join().unwrap();
    assert!(sent.contains("x-supplied: yes\r\n"), "{sent}");
    assert!(clients.async_clients.is_empty());
}

#[tokio::test]
async fn client_uses_the_supplied_async_client_for_versions() {
    let serving = server::serve_once(server::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::default();
    let mut clients = clients();
    let mut logging = Logging::default();
    let call = call_with_unusable_ssl_verify(serving.address);
    let supplied = base_builder()
        .default_headers(marker_headers())
        .build()
        .unwrap();

    let output = async_list_agent_versions(
        &fixture,
        &mut clients,
        &mut logging,
        Some(supplied),
        &call.agent_request(),
    )
    .await
    .unwrap();

    assert_eq!(output, "versions|agent-1|200|2");
    let sent = serving.request.join().unwrap();
    assert!(sent.contains("x-supplied: yes\r\n"), "{sent}");
    assert!(clients.async_clients.is_empty());
}
