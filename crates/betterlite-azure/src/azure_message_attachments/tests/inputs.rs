#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_http::client_kind::ClientKind;
use secrecy::{ExposeSecret, SecretString};

use crate::azure_assistant_definitions::error::DefinitionsError;
use crate::azure_message_attachments::error::MessagesError;
use crate::azure_message_attachments::operations::{MessageCall, MessageReply};
use crate::credentials::source::CallableTokenProvider;
use crate::endpoints::projection::{AzureClientInputs, CallableProvider};

use super::support::{
    Auth, EmptyToken, FailingToken, FixedToken, Fixture, KEY, MESSAGE, MESSAGE_DATA, MESSAGES_URL,
    PAGE, Recorder, client_handle, data, harness, wire,
};

pub(super) async fn check() {
    litellm_params_win().await;
    async_client_inputs();
    supplied_client_kept().await;
    environment_key_and_endpoint().await;
    providers().await;
    missing();
    empty_key();
}

// Values inside litellm_params reach the initializer; the facade's own arguments never do.
async fn litellm_params_win() {
    let mut fixture = Fixture::new();
    fixture.api_key = Some(SecretString::from("arg-key"));
    fixture.api_base = Some(String::from("https://arg.test"));
    fixture.api_version = Some(String::from("2024-05-01"));
    fixture.azure_ad_token = Some(SecretString::from("method-token"));
    fixture.timeout = 5.0_f64;
    fixture.max_retries = Some(9);
    fixture.params.azure_ad_token = Some(SecretString::from("params-token"));
    fixture.params.timeout = Some(30.0_f64);
    fixture.params.max_retries = Some(4);
    let request = fixture.request();
    let data = data(MESSAGE_DATA);
    let (api, transport) = harness(200, MESSAGE);
    let client = api
        .assistants
        .azure_client(&request, None, ClientKind::Sync)
        .unwrap();
    assert_param_inputs(client.inputs.as_ref().unwrap());
    let reply = api
        .call(
            &request,
            None,
            None,
            MessageCall::Append {
                thread_id: "thread_1",
                data: &data,
            },
        )
        .await
        .unwrap();
    let MessageReply::Message(message) = reply.reply else {
        panic!("append returns a message");
    };
    assert!(message.status.is_some());
    assert_params_request(&transport);
}

fn assert_params_request(transport: &Recorder) {
    let mut expected = wire(
        "POST",
        "https://arg.test/openai/threads/thread_1/messages?api-version=2024-05-01",
        Some(MESSAGE_DATA),
        Auth::Bearer("params-token"),
    );
    expected.deadline = Duration::from_secs(30);
    assert_eq!(transport.requests(), vec![expected]);
}

fn assert_param_inputs(inputs: &AzureClientInputs<'_>) {
    assert_eq!(inputs.api_version, "2024-05-01");
    assert_eq!(inputs.max_retries, 4);
    assert_eq!(inputs.timeout.map(f64::to_bits), Some(30.0_f64.to_bits()));
    assert_eq!(
        inputs.endpoint.azure_endpoint.as_deref(),
        Some("https://arg.test")
    );
    assert_eq!(
        inputs.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("arg-key")
    );
    assert_eq!(
        inputs
            .azure_ad_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("params-token")
    );
    assert!(!inputs.http_client.is_async);
}

fn async_client_inputs() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let (api, _transport) = harness(200, MESSAGE);
    let client = api
        .assistants
        .azure_client(&request, None, ClientKind::Asynchronous)
        .unwrap();
    assert_eq!(client.kind, ClientKind::Asynchronous);
    assert!(client.inputs.as_ref().unwrap().http_client.is_async);
}

// A supplied client wins even when construction would fail, keeps its own kind, and serves the request.
async fn supplied_client_kept() {
    let supplied_transport = Arc::new(Recorder::default());
    supplied_transport.queue(200, MESSAGE);
    let mut fixture = Fixture::new();
    fixture.api_key = None;
    fixture.api_base = None;
    let request = fixture.request();
    let data = data(MESSAGE_DATA);
    let (api, api_transport) = harness(200, MESSAGE);
    let blocked = api
        .assistants
        .azure_client(&request, None, ClientKind::Sync);
    assert!(matches!(blocked, Err(DefinitionsError::MissingCredentials)));
    let supplied = client_handle(ClientKind::Sync, Arc::clone(&supplied_transport));
    let reply = api
        .call(
            &request,
            Some(supplied),
            Some(true),
            MessageCall::Append {
                thread_id: "thread_1",
                data: &data,
            },
        )
        .await
        .unwrap();
    assert_eq!(reply.kind, ClientKind::Sync);
    let mut expected = wire(
        "POST",
        "https://supplied.test/openai/threads/thread_1/messages?api-version=2023-05-01",
        Some(MESSAGE_DATA),
        Auth::Key("supplied-key"),
    );
    expected.deadline = Duration::from_secs(42);
    assert_eq!(supplied_transport.requests(), vec![expected]);
    assert!(api_transport.requests().is_empty());
}

async fn environment_key_and_endpoint() {
    let mut fixture = Fixture::new();
    fixture.api_key = None;
    fixture.api_base = None;
    fixture.environment.insert(
        String::from("AZURE_OPENAI_API_KEY"),
        String::from("env-key"),
    );
    fixture.environment.insert(
        String::from("AZURE_OPENAI_ENDPOINT"),
        String::from("https://env.test/"),
    );
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    api.call(&request, None, None, MessageCall::List("thread_1"))
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire(
            "GET",
            "https://env.test/openai/threads/thread_1/messages?api-version=2024-02-01",
            None,
            Auth::Key("env-key")
        )]
    );
}

// A provider without a key authenticates with its bearer alone; next to a key it sends both.
async fn providers() {
    provider_alone().await;
    provider_with_key().await;
    static_token_wins().await;
    provider_failures().await;
}

async fn provider_alone() {
    let mut fixture = Fixture::new();
    fixture.api_key = None;
    fixture.provider = Some(provider(Arc::new(FixedToken("provider-token"))));
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    api.call(&request, None, None, MessageCall::List("thread_1"))
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire(
            "GET",
            MESSAGES_URL,
            None,
            Auth::Bearer("provider-token")
        )]
    );
}

async fn provider_with_key() {
    let mut fixture = Fixture::new();
    fixture.provider = Some(provider(Arc::new(FixedToken("provider-token"))));
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    api.call(&request, None, None, MessageCall::List("thread_1"))
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire(
            "GET",
            MESSAGES_URL,
            None,
            Auth::Both {
                key: KEY,
                bearer: "provider-token"
            }
        )]
    );
}

async fn static_token_wins() {
    let mut fixture = Fixture::new();
    fixture.params.azure_ad_token = Some(SecretString::from("static-token"));
    fixture.provider = Some(provider(Arc::new(FailingToken)));
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    api.call(&request, None, None, MessageCall::List("thread_1"))
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire(
            "GET",
            MESSAGES_URL,
            None,
            Auth::Bearer("static-token")
        )]
    );
}

// A provider failure and a provider reply that is not text surface as their own errors before any request.
async fn provider_failures() {
    let tokens: [(Arc<dyn CallableTokenProvider>, bool); 2] = [
        (Arc::new(FailingToken), false),
        (Arc::new(EmptyToken), true),
    ];
    for (token, reply_expected) in tokens {
        let mut fixture = Fixture::new();
        fixture.api_key = None;
        fixture.provider = Some(provider(token));
        let request = fixture.request();
        let (api, transport) = harness(200, PAGE);
        let error = api
            .call(&request, None, None, MessageCall::List("thread_1"))
            .await
            .unwrap_err();
        if reply_expected {
            assert!(matches!(
                error,
                MessagesError::Definitions(DefinitionsError::ProviderReply)
            ));
        } else {
            assert!(matches!(
                &error,
                MessagesError::Definitions(DefinitionsError::Provider { class, .. })
                    if class == "ClientAuthenticationError"
            ));
        }
        assert!(transport.requests().is_empty());
    }
}

fn provider(token: Arc<dyn CallableTokenProvider>) -> CallableProvider {
    CallableProvider {
        provider: token,
        identity: 4,
    }
}

// Both missing inputs refuse before any request, each with its own error.
fn missing() {
    let mut fixture = Fixture::new();
    fixture.api_key = None;
    let request = fixture.request();
    let (api, transport) = harness(200, MESSAGE);
    let refused = api
        .assistants
        .azure_client(&request, None, ClientKind::Sync);
    assert!(matches!(refused, Err(DefinitionsError::MissingCredentials)));
    assert!(transport.requests().is_empty());
    let mut endpointless = Fixture::new();
    endpointless.api_base = None;
    let endpoint_request = endpointless.request();
    let (endpoint_api, endpoint_transport) = harness(200, MESSAGE);
    let endpointless_refused =
        endpoint_api
            .assistants
            .azure_client(&endpoint_request, None, ClientKind::Sync);
    assert!(matches!(
        endpointless_refused,
        Err(DefinitionsError::MissingEndpoint)
    ));
    assert!(endpoint_transport.requests().is_empty());
}

// The SDK's base client rejects an empty key before any token or provider is considered.
fn empty_key() {
    let mut fixture = Fixture::new();
    fixture.api_key = Some(SecretString::from(""));
    fixture.params.azure_ad_token = Some(SecretString::from("params-token"));
    let request = fixture.request();
    let (api, transport) = harness(200, MESSAGE);
    let refused = api
        .assistants
        .azure_client(&request, None, ClientKind::Sync);
    assert!(matches!(refused, Err(DefinitionsError::MissingCredentials)));
    assert!(transport.requests().is_empty());
}
