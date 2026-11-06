#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};

use crate::assistants::client::{ClientKind, ClientOptions, SdkClient};
use crate::assistants::records::{AssistantCreate, ListParams};

use crate::assistants::operations::AssistantsApi;

use super::support::{ASSISTANT, DELETED, KEY, PAGE, Recorder, api_with, options};

// The client carries exactly the keyword arguments the OpenAI constructor would get.
pub(super) fn client_argument_projection() {
    let (api, _transport) = api_with(200, PAGE);
    let bare = api.get_openai_client(ClientKind::Sync, ClientOptions::default(), None);
    assert_eq!(bare.kind, ClientKind::Sync);
    assert!(bare.api_key.is_none() && bare.base_url.is_none() && bare.timeout.is_none());
    assert!(bare.max_retries.is_none() && bare.organization.is_none());
    forwarded_options(&api);
    empty_options_are_kept(&api);
    supplied_client_wins(&api);
}

// Given values pass through untouched, and empty or zero values are kept rather than dropped.
fn forwarded_options(api: &AssistantsApi<Recorder>) {
    let full = ClientOptions {
        api_key: Some(SecretString::from("k")),
        api_base: Some(String::from("https://h/v1")),
        timeout: Some(Duration::from_millis(2500)),
        max_retries: Some(3),
        organization: Some(String::from("o")),
    };
    let client = api.get_openai_client(ClientKind::Async, full, None);
    assert_eq!(client.kind, ClientKind::Async);
    let seen = (
        client.api_key.as_ref().map(ExposeSecret::expose_secret),
        client.base_url.as_deref(),
        client.timeout,
        client.max_retries,
        client.organization.as_deref(),
    );
    let timeout = Duration::from_millis(2500);
    assert_eq!(
        seen,
        (
            Some("k"),
            Some("https://h/v1"),
            Some(timeout),
            Some(3),
            Some("o")
        )
    );
}

fn empty_options_are_kept(api: &AssistantsApi<Recorder>) {
    let empty = ClientOptions {
        api_key: Some(SecretString::from("")),
        api_base: Some(String::new()),
        timeout: Some(Duration::ZERO),
        max_retries: Some(0),
        organization: Some(String::new()),
    };
    let kept = api.get_openai_client(ClientKind::Sync, empty, None);
    assert_eq!(kept.api_key.map(|key| key.expose_secret().len()), Some(0));
    assert_eq!(kept.base_url, Some(String::new()));
    assert_eq!(kept.timeout, Some(Duration::ZERO));
    assert_eq!(kept.max_retries, Some(0));
    assert_eq!(kept.organization, Some(String::new()));
}

fn supplied_client_wins(api: &AssistantsApi<Recorder>) {
    let supplied = SdkClient {
        kind: ClientKind::Async,
        api_key: None,
        base_url: None,
        timeout: None,
        max_retries: None,
        organization: Some(String::from("mine")),
        transport: Arc::clone(&api.transport),
    };
    let chosen = api.get_openai_client(ClientKind::Sync, options("other"), Some(supplied));
    assert_eq!(chosen.kind, ClientKind::Async);
    assert_eq!(chosen.organization.as_deref(), Some("mine"));
    assert!(chosen.api_key.is_none() && chosen.base_url.is_none());
}

// Every call carries OpenAI-Beta: assistants=v2 although the caller never sets it.
pub(super) async fn beta_header() {
    let (api, transport) = api_with(200, PAGE);
    transport.queue(200, ASSISTANT);
    transport.queue(200, DELETED);
    let create: AssistantCreate = serde_json::from_str(r#"{"model":"gpt-4o"}"#).unwrap();
    api.get_assistants(options(KEY), None, false, &ListParams::default())
        .await
        .unwrap();
    api.create_assistants(options(KEY), &create, None, false)
        .await
        .unwrap();
    api.delete_assistant(options(KEY), "asst_1", None, false)
        .await
        .unwrap();
    let betas: Vec<_> = transport
        .requests()
        .into_iter()
        .map(|seen| seen.beta)
        .collect();
    assert_eq!(betas, vec![Some(String::from("assistants=v2")); 3]);
}
