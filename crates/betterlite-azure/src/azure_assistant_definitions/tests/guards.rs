#![cfg(test)]

use std::sync::Arc;

use betterlite_config::sections::azure_credentials::DEFAULT_SCOPE;
use betterlite_http::client_kind::ClientKind;
use betterlite_openai::assistants::error::AssistantsError;
use secrecy::SecretString;

use crate::azure_assistant_definitions::error::DefinitionsError;
use crate::azure_assistant_definitions::operations::DefinitionCall;
use crate::endpoints::projection::CallableProvider;

use super::support::{
    ERROR_BODY, FixedToken, Fixture, KEY, PAGE, Recorder, Seen, failing_harness, harness,
};

const KEY_A: &str = "sk-identity-a";
const KEY_B: &str = "sk-identity-b";
const TOKEN_A: &str = "token-identity-a";
const TOKEN_B: &str = "token-identity-b";

// Two configured identities never share a key, a bearer or a cached provider.
#[tokio::test]
async fn azure_assistant_definitions_credential_identity_isolation() {
    key_identities().await;
    provider_identities().await;
    refresh_identities().await;
}

async fn key_identities() {
    let mut first = Fixture::new();
    first.api_key = Some(SecretString::from(KEY_A));
    let second = Fixture::new();
    let (first_seen, second_seen) = two_lists(&first, &second).await;
    assert_eq!(
        first_seen.headers.get("api-key").map(String::as_str),
        Some(KEY_A)
    );
    assert_eq!(
        second_seen.headers.get("api-key").map(String::as_str),
        Some(KEY)
    );
    assert!(!first_seen.headers.contains_key("authorization"));
    assert!(!second_seen.headers.contains_key("authorization"));
    assert!(
        !first_seen
            .headers
            .values()
            .any(|value| value.contains(KEY_B))
    );
    assert!(
        !second_seen
            .headers
            .values()
            .any(|value| value.contains(KEY_A))
    );
}

async fn provider_identities() {
    let mut first = Fixture::new();
    first.api_key = Some(SecretString::from(KEY_A));
    first.provider = Some(provider(TOKEN_A, 1));
    let mut second = Fixture::new();
    second.api_key = Some(SecretString::from(KEY_B));
    second.provider = Some(provider(TOKEN_B, 2));
    let (first_seen, second_seen) = two_lists(&first, &second).await;
    assert_eq!(
        first_seen.headers.get("api-key").map(String::as_str),
        Some(KEY_A)
    );
    assert_eq!(
        second_seen.headers.get("api-key").map(String::as_str),
        Some(KEY_B)
    );
    assert_eq!(
        first_seen.headers.get("authorization").map(String::as_str),
        Some("Bearer token-identity-a")
    );
    assert_eq!(
        second_seen.headers.get("authorization").map(String::as_str),
        Some("Bearer token-identity-b")
    );
}

async fn refresh_identities() {
    let mut fixture = Fixture::new();
    fixture.api_key = None;
    fixture.refresh_enabled = true;
    fixture.sources.script(Arc::new(FixedToken(TOKEN_A)));
    fixture.sources.script(Arc::new(FixedToken(TOKEN_B)));
    let first_request = fixture.request();
    let (first_api, first_transport) = harness(200, PAGE);
    let first_client = first_api
        .azure_client(&first_request, None, ClientKind::Sync)
        .unwrap();
    first_client.list().await.unwrap();
    let second_request = fixture.request();
    let (second_api, second_transport) = harness(200, PAGE);
    let second_client = second_api
        .azure_client(&second_request, None, ClientKind::Sync)
        .unwrap();
    second_client.list().await.unwrap();
    let first_seen = one_request(&first_transport);
    let second_seen = one_request(&second_transport);
    assert_eq!(
        first_seen.headers.get("authorization").map(String::as_str),
        Some("Bearer token-identity-a")
    );
    assert_eq!(
        second_seen.headers.get("authorization").map(String::as_str),
        Some("Bearer token-identity-b")
    );
    assert!(!first_seen.headers.contains_key("api-key"));
    assert_eq!(
        fixture.sources.refresh_scopes(),
        vec![String::from(DEFAULT_SCOPE), String::from(DEFAULT_SCOPE)]
    );
}

async fn two_lists(first: &Fixture, second: &Fixture) -> (Seen, Seen) {
    let first_request = first.request();
    let (first_api, first_transport) = harness(200, PAGE);
    let first_client = first_api
        .azure_client(&first_request, None, ClientKind::Sync)
        .unwrap();
    first_client.list().await.unwrap();
    let second_request = second.request();
    let (second_api, second_transport) = harness(200, PAGE);
    let second_client = second_api
        .azure_client(&second_request, None, ClientKind::Sync)
        .unwrap();
    second_client.list().await.unwrap();
    (
        one_request(&first_transport),
        one_request(&second_transport),
    )
}

fn provider(token: &'static str, identity: u64) -> CallableProvider {
    CallableProvider {
        provider: Arc::new(FixedToken(token)),
        identity,
    }
}

fn one_request(transport: &Arc<Recorder>) -> Seen {
    let mut seen = transport.requests();
    assert_eq!(seen.len(), 1);
    seen.remove(0)
}

// Debug output and the owned failure paths carry no credential.
#[tokio::test]
async fn azure_assistant_definitions_secret_safe_failure() {
    debug_hides_credentials();
    status_reply_hides_credentials().await;
    failure_hides_credentials().await;
}

fn debug_hides_credentials() {
    let mut keyed = Fixture::new();
    keyed.api_key = Some(SecretString::from(KEY));
    let (keyed_api, _keyed_transport) = harness(200, PAGE);
    let keyed_request = keyed.request();
    let keyed_client = keyed_api
        .azure_client(&keyed_request, None, ClientKind::Sync)
        .unwrap();
    let shown = format!("{keyed_client:?}");
    assert!(shown.starts_with("AzureAssistantsClient"), "{shown}");
    assert_clean("client debug", &shown);
    let mut tokened = Fixture::new();
    tokened.params.azure_ad_token = Some(SecretString::from(TOKEN_A));
    let (tokened_api, _tokened_transport) = harness(200, PAGE);
    let tokened_request = tokened.request();
    let tokened_client = tokened_api
        .azure_client(&tokened_request, None, ClientKind::Sync)
        .unwrap();
    assert_clean("client debug", &format!("{tokened_client:?}"));
}

async fn status_reply_hides_credentials() {
    let mut fixture = Fixture::new();
    fixture.params.azure_ad_token = Some(SecretString::from(TOKEN_A));
    let request = fixture.request();
    let (api, transport) = harness(401, ERROR_BODY);
    let failed = api
        .call(&request, None, None, DefinitionCall::List)
        .await
        .unwrap_err();
    assert!(
        matches!(
            failed,
            DefinitionsError::Assistants(AssistantsError::Status { status: 401, .. })
        ),
        "{failed:?}"
    );
    assert_clean("status display", &failed.to_string());
    assert_clean("status debug", &format!("{failed:?}"));
    let shown = transport.debug_texts();
    assert_eq!(shown.len(), 1);
    assert!(shown[0].contains("Sensitive"), "{}", shown[0]);
    assert_clean("request debug", &shown[0]);
}

// A transport fault and a credential the SDK cannot turn into a header are both named, never echoed.
async fn failure_hides_credentials() {
    let mut tokened = Fixture::new();
    tokened.params.azure_ad_token = Some(SecretString::from(TOKEN_A));
    let tokened_request = tokened.request();
    let (tokened_api, tokened_transport) = failing_harness();
    let transport_failure = tokened_api
        .call(&tokened_request, None, None, DefinitionCall::List)
        .await
        .unwrap_err();
    assert!(
        matches!(
            transport_failure,
            DefinitionsError::Assistants(AssistantsError::Transport { .. })
        ),
        "{transport_failure:?}"
    );
    assert_named_clean("transport", &transport_failure);
    for shown in tokened_transport.debug_texts() {
        assert_clean("transport request debug", &shown);
    }
    let mut bad_header = Fixture::new();
    bad_header.api_key = Some(SecretString::from("sk-fixture\nbad"));
    let header_request = bad_header.request();
    let (header_api, header_transport) = harness(200, PAGE);
    let header_failure = header_api
        .call(&header_request, None, None, DefinitionCall::List)
        .await
        .unwrap_err();
    assert!(
        matches!(
            header_failure,
            DefinitionsError::Assistants(AssistantsError::Header { .. })
        ),
        "{header_failure:?}"
    );
    assert_named_clean("header", &header_failure);
    assert!(header_transport.requests().is_empty());
}

fn assert_named_clean(what: &str, failed: &DefinitionsError) {
    assert_clean(&format!("{what} display"), &failed.to_string());
    assert_clean(&format!("{what} debug"), &format!("{failed:?}"));
}

fn assert_clean(what: &str, text: &str) {
    for secret in [KEY, TOKEN_A] {
        assert!(!text.contains(secret), "{what}: {text}");
    }
}
