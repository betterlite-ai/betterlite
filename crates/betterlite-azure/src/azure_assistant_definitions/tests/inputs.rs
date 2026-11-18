#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_openai::assistants::client::ClientKind;
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_assistant_definitions::client::AzureAssistantsClient;
use crate::azure_assistant_definitions::error::DefinitionsError;
use crate::credentials::source::CallableTokenProvider;
use crate::endpoints::projection::{AzureClientInputs, CallableProvider};

use super::support::{
    ASSISTANTS_URL, Auth, EmptyToken, FailingToken, FixedToken, Fixture, KEY, PAGE, Recorder,
    harness, wire,
};

pub(super) async fn check() {
    litellm_params_win().await;
    litellm_params_absent().await;
    async_client_inputs();
    supplied_sync_kept().await;
    supplied_async_kept();
    environment_key_and_endpoint().await;
    environment_token().await;
    missing_credentials();
    empty_key();
    missing_endpoint();
    negative_timeout();
    provider_only().await;
    provider_headers().await;
    static_token_wins().await;
    provider_failures().await;
    deployments_base().await;
}

// Values inside litellm_params reach the initializer; the facade's own arguments never do.
async fn litellm_params_win() {
    let mut fixture = Fixture::new();
    fixture.api_key = Some(SecretString::from("arg-key"));
    fixture.api_base = Some(String::from("https://arg.test"));
    fixture.api_version = Some(String::from("2024-05-01"));
    fixture.azure_ad_token = Some(SecretString::from("method-token"));
    fixture.timeout = 5.0;
    fixture.max_retries = Some(9);
    fixture.params.azure_ad_token = Some(SecretString::from("params-token"));
    fixture.params.timeout = Some(30.0);
    fixture.params.max_retries = Some(4);
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    let client = api.get_azure_client(&request, None).unwrap();
    assert_param_inputs(client.inputs.as_ref().unwrap());
    assert_eq!(client.kind, ClientKind::Sync);
    client.list().await.unwrap();
    let mut expected = wire(
        "GET",
        "https://arg.test/openai/assistants?api-version=2024-05-01",
        None,
        Auth::Bearer("params-token"),
    );
    expected.deadline = Duration::from_secs(30);
    assert_eq!(transport.requests(), vec![expected]);
}

fn assert_param_inputs(inputs: &AzureClientInputs<'_>) {
    assert_eq!(inputs.api_version, "2024-05-01");
    assert_eq!(inputs.max_retries, 4);
    assert_eq!(inputs.timeout.map(f64::to_bits), Some(30.0f64.to_bits()));
    assert_eq!(
        inputs.endpoint.azure_endpoint.as_deref(),
        Some("https://arg.test")
    );
    assert_eq!(
        inputs
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("arg-key")
    );
    assert_eq!(
        inputs
            .azure_ad_token
            .as_ref()
            .map(|token| token.expose_secret().as_str()),
        Some("params-token")
    );
    assert!(!inputs.http_client.is_async);
}

async fn litellm_params_absent() {
    let mut fixture = Fixture::new();
    fixture.azure_ad_token = Some(SecretString::from("method-token"));
    fixture.timeout = 5.0;
    fixture.max_retries = Some(9);
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    let client = api.get_azure_client(&request, None).unwrap();
    let inputs = client.inputs.as_ref().unwrap();
    assert_eq!(inputs.max_retries, 2);
    assert_eq!(inputs.timeout, None);
    assert!(inputs.azure_ad_token.is_none());
    client.list().await.unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire("GET", ASSISTANTS_URL, None, Auth::Key(KEY))]
    );
}

fn async_client_inputs() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let (api, _transport) = harness(200, PAGE);
    let client = api.async_get_azure_client(&request, None).unwrap();
    assert_eq!(client.kind, ClientKind::Async);
    assert!(client.inputs.as_ref().unwrap().http_client.is_async);
}

// A supplied client wins even when construction would fail, keeps its own kind, and serves the request.
async fn supplied_sync_kept() {
    let supplied_transport = Arc::new(Recorder::default());
    supplied_transport.queue(200, PAGE);
    let mut fixture = Fixture::new();
    fixture.api_key = None;
    fixture.api_base = None;
    let request = fixture.request();
    let (api, api_transport) = harness(200, PAGE);
    let blocked = api.get_azure_client(&request, None);
    assert!(matches!(blocked, Err(DefinitionsError::MissingCredentials)));
    let supplied = client_handle(ClientKind::Sync, Arc::clone(&supplied_transport));
    let kept = api
        .async_get_azure_client(&request, Some(supplied))
        .unwrap();
    assert_eq!(kept.kind, ClientKind::Sync);
    assert!(kept.inputs.is_none());
    kept.list().await.unwrap();
    let seen = supplied_transport.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].url,
        "https://supplied.test/openai/assistants?api-version=2024-01-01"
    );
    assert_eq!(seen[0].deadline, Duration::from_secs(42));
    assert_eq!(
        seen[0].headers.get("api-key").map(String::as_str),
        Some("supplied-key")
    );
    assert!(!seen[0].headers.contains_key("authorization"));
    assert!(api_transport.requests().is_empty());
}

fn supplied_async_kept() {
    let supplied_transport = Arc::new(Recorder::default());
    let mut fixture = Fixture::new();
    fixture.api_key = None;
    let request = fixture.request();
    let (api, _transport) = harness(200, PAGE);
    let supplied = client_handle(ClientKind::Async, Arc::clone(&supplied_transport));
    let kept = api.get_azure_client(&request, Some(supplied)).unwrap();
    assert_eq!(kept.kind, ClientKind::Async);
    assert!(kept.inputs.is_none());
    assert!(supplied_transport.requests().is_empty());
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
    api.get_azure_client(&request, None)
        .unwrap()
        .list()
        .await
        .unwrap();
    let expected = wire(
        "GET",
        "https://env.test/openai/assistants?api-version=2024-02-01",
        None,
        Auth::Key("env-key"),
    );
    assert_eq!(transport.requests(), vec![expected]);
}

async fn environment_token() {
    let mut fixture = Fixture::new();
    fixture.api_key = None;
    fixture.environment.insert(
        String::from("AZURE_OPENAI_AD_TOKEN"),
        String::from("env-token"),
    );
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    api.get_azure_client(&request, None)
        .unwrap()
        .list()
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire("GET", ASSISTANTS_URL, None, Auth::Bearer("env-token"))]
    );
}

fn missing_credentials() {
    let mut fixture = Fixture::new();
    fixture.api_key = None;
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    let refused = api.get_azure_client(&request, None);
    assert!(matches!(refused, Err(DefinitionsError::MissingCredentials)));
    assert!(transport.requests().is_empty());
}

// The SDK's base client rejects an empty key before any token is considered.
fn empty_key() {
    let mut fixture = Fixture::new();
    fixture.api_key = Some(SecretString::from(""));
    fixture.params.azure_ad_token = Some(SecretString::from("params-token"));
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    let refused = api.get_azure_client(&request, None);
    assert!(matches!(refused, Err(DefinitionsError::MissingCredentials)));
    assert!(transport.requests().is_empty());
}

fn missing_endpoint() {
    let mut fixture = Fixture::new();
    fixture.api_base = None;
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    let refused = api.get_azure_client(&request, None);
    assert!(matches!(refused, Err(DefinitionsError::MissingEndpoint)));
    assert!(transport.requests().is_empty());
}

fn negative_timeout() {
    let mut fixture = Fixture::new();
    fixture.params.timeout = Some(-1.0);
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    let refused = api.get_azure_client(&request, None);
    assert!(matches!(refused, Err(DefinitionsError::Timeout { .. })));
    assert!(transport.requests().is_empty());
}

// A provider without a key authenticates with its bearer alone.
async fn provider_only() {
    let mut fixture = Fixture::new();
    fixture.api_key = None;
    fixture.provider = Some(CallableProvider {
        provider: Arc::new(FixedToken("provider-token")),
        identity: 4,
    });
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    api.get_azure_client(&request, None)
        .unwrap()
        .list()
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire(
            "GET",
            ASSISTANTS_URL,
            None,
            Auth::Bearer("provider-token")
        )]
    );
}

// An empty token from the environment is a token the SDK sends as an empty bearer.
async fn empty_token() {
    let mut fixture = Fixture::new();
    fixture.api_key = None;
    fixture
        .environment
        .insert(String::from("AZURE_OPENAI_AD_TOKEN"), String::new());
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    api.get_azure_client(&request, None)
        .unwrap()
        .list()
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire("GET", ASSISTANTS_URL, None, Auth::Bearer(""))]
    );
}

async fn provider_headers() {
    let mut fixture = Fixture::new();
    fixture.provider = Some(CallableProvider {
        provider: Arc::new(FixedToken("provider-token")),
        identity: 1,
    });
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    api.get_azure_client(&request, None)
        .unwrap()
        .list()
        .await
        .unwrap();
    let expected = wire(
        "GET",
        ASSISTANTS_URL,
        None,
        Auth::Both {
            key: KEY,
            bearer: "provider-token",
        },
    );
    assert_eq!(transport.requests(), vec![expected]);
}

async fn static_token_wins() {
    let mut fixture = Fixture::new();
    fixture.params.azure_ad_token = Some(SecretString::from("static-token"));
    fixture.provider = Some(CallableProvider {
        provider: Arc::new(FailingToken),
        identity: 2,
    });
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    api.get_azure_client(&request, None)
        .unwrap()
        .list()
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire(
            "GET",
            ASSISTANTS_URL,
            None,
            Auth::Bearer("static-token")
        )]
    );
}

async fn provider_failures() {
    let failed = provider_error(Arc::new(FailingToken)).await;
    assert!(
        matches!(failed, DefinitionsError::Provider { ref class, .. } if class == "ClientAuthenticationError"),
        "{failed:?}"
    );
    let empty = provider_error(Arc::new(EmptyToken)).await;
    assert!(
        matches!(empty, DefinitionsError::ProviderReply),
        "{empty:?}"
    );
}

async fn provider_error(provider: Arc<dyn CallableTokenProvider>) -> DefinitionsError {
    let mut fixture = Fixture::new();
    fixture.provider = Some(CallableProvider {
        provider,
        identity: 3,
    });
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    let error = api
        .get_azure_client(&request, None)
        .unwrap()
        .list()
        .await
        .unwrap_err();
    assert!(transport.requests().is_empty());
    error
}

async fn deployments_base() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::from("https://example.test/openai/deployments/dep"));
    let request = fixture.request();
    let (api, transport) = harness(200, PAGE);
    let client = api.get_azure_client(&request, None).unwrap();
    let endpoint = &client.inputs.as_ref().unwrap().endpoint;
    assert_eq!(
        endpoint.base_url.as_deref(),
        Some("https://example.test/openai/deployments/dep")
    );
    assert_eq!(endpoint.azure_endpoint, None);
    client.list().await.unwrap();
    let expected = wire(
        "GET",
        "https://example.test/openai/deployments/dep/assistants?api-version=2024-02-01",
        None,
        Auth::Key(KEY),
    );
    assert_eq!(transport.requests(), vec![expected]);
}

fn client_handle(
    kind: ClientKind,
    transport: Arc<Recorder>,
) -> AzureAssistantsClient<'static, Recorder> {
    AzureAssistantsClient {
        kind,
        inputs: None,
        transport,
        base_url: String::from("https://supplied.test/openai"),
        api_version: String::from("2024-01-01"),
        key: Some(SecretString::from("supplied-key")),
        token: None,
        deadline: Duration::from_secs(42),
    }
}
