#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::{azure_endpoints, resource_sdk_settings};
use betterlite_error::envelope::WireField;
use betterlite_foundry::chat::sources::NoSources;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::assistants::records::AssistantCreate;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::HeaderMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;
use tokio::task::yield_now;

use super::call::{
    AssistantCreateCall, AssistantCreateOutcome, AssistantCreateResult, AzureCreateCall,
    OpenAiCreateCall,
};
use super::error::{AssistantCreateFailure, Entry, Raised, no_response, raised};
use super::service::AssistantRecords;
use super::{AssistantCreateInput, AsyncFlag, ClientClass, CreateTimeout};
use crate::api::resource_sdk_settings::timeout::FineTuningTimeout;

const ASSISTANT: &str = r#"{"id":"asst_a","object":"assistant","model":"gpt-4o-mini","created_at":1,"name":null,"tools":[]}"#;

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    url: String,
    headers: BTreeMap<String, String>,
    body: AssistantCreate,
    deadline: Duration,
}

#[derive(Default)]
struct Scripted {
    replies: Mutex<Vec<UpstreamResponse>>,
    seen: Mutex<Vec<Seen>>,
}

impl Scripted {
    fn reply(self: &Arc<Self>, status: u16, body: &str) {
        self.reply_bytes(status, body.as_bytes().to_vec());
    }

    fn reply_bytes(self: &Arc<Self>, status: u16, body: Vec<u8>) {
        self.replies
            .lock()
            .expect("replies")
            .push(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body,
            });
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().expect("seen").clone()
    }
}

fn readable_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().expect("a readable header").to_owned(),
            )
        })
        .collect()
}

impl AssistantsTransport for Scripted {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        // the mock answers from memory; this await keeps it a real asynchronous transport
        yield_now().await;
        let RequestBody::Json(bytes) = &request.body else {
            panic!("the create request carries a JSON body");
        };
        let body = serde_json::from_slice::<AssistantCreate>(bytes)
            .expect("the create body is an AssistantCreate");
        let headers = readable_headers(&request.headers);
        let mut url = request.url.to_string();
        if !request.query.is_empty() {
            let pairs: Vec<String> = request
                .query
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect();
            url.push('?');
            url.push_str(&pairs.join("&"));
        }
        self.seen.lock().expect("seen").push(Seen {
            method: request.method.as_str().to_owned(),
            url,
            headers,
            body,
            deadline: request.deadline,
        });
        let mut replies = self.replies.lock().expect("replies");
        if replies.is_empty() {
            return Ok(UpstreamResponse {
                status: 200,
                headers: HeaderMap::new(),
                body: ASSISTANT.as_bytes().to_vec(),
            });
        }
        Ok(replies.remove(0))
    }
}

struct Fixture {
    settings: resource_sdk_settings::Settings,
    azure: azure_endpoints::Settings,
    environment: BTreeMap<String, String>,
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    transport: Arc<Scripted>,
}

impl Fixture {
    fn new(settings: resource_sdk_settings::Settings) -> Self {
        Self {
            settings,
            azure: azure_endpoints::Settings::default(),
            environment: BTreeMap::new(),
            catalog: ModelCatalog::default(),
            names: BTreeSet::new(),
            transport: Arc::default(),
        }
    }

    fn env(mut self, name: &str, value: &str) -> Self {
        self.environment.insert(name.to_owned(), value.to_owned());
        self
    }

    fn reply(self, status: u16, body: &str) -> Self {
        self.transport.reply(status, body);
        self
    }

    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    fn service<'fixture, 'resolution>(
        &'fixture self,
        resolution: &'resolution ResolutionContext<'fixture>,
        redactor: &'fixture Redactor,
    ) -> AssistantRecords<'fixture, 'resolution, Scripted> {
        AssistantRecords {
            settings: self.settings.clone(),
            azure: self.azure.clone(),
            environment: &self.environment,
            sources: &NoSources,
            resolution,
            redactor,
            transport: Arc::clone(&self.transport),
        }
    }
}

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("a redactor")
}

// a JSON fixture kept as written, so a request body reads like the document LiteLLM's caller sends
fn raw(document: &str) -> Box<RawValue> {
    RawValue::from_string(document.to_owned()).expect("a JSON fixture")
}

fn input<Fields>(extra: Fields) -> AssistantCreateInput
where
    Fields: serde::Serialize,
{
    let mut base = serde_json::to_value(BTreeMap::from([
        ("provider", "openai"),
        ("model", "gpt-4o-mini"),
    ]))
    .expect("the base input");
    let fields = serde_json::to_value(extra).expect("the extra fields are JSON");
    if let (Some(target), Some(source)) = (base.as_object_mut(), fields.as_object()) {
        for (key, value) in source {
            target.insert(key.clone(), value.clone());
        }
    }
    serde_json::from_value(base).expect("an assistant create input")
}

fn create_body<Fields>(fields: Fields) -> AssistantCreate
where
    Fields: serde::Serialize,
{
    let value = serde_json::to_value(fields).expect("the expected fields are JSON");
    serde_json::from_value(value).expect("an assistant create body")
}

// a config with nothing set, so a test that sets two values says which two
fn bare_settings() -> resource_sdk_settings::Settings {
    let lookup = BTreeMap::<String, String>::new();
    let config = ConfigLoader { lookup }.load("{}").expect("an empty config");
    config.sections.resource_sdk_settings
}

fn openai_settings(api_key: &str) -> resource_sdk_settings::Settings {
    let mut settings = bare_settings();
    settings.api_key = Some(SecretString::from(api_key));
    settings
}

fn openai_call(result: &AssistantCreateResult) -> &OpenAiCreateCall {
    match &result.call {
        AssistantCreateCall::OpenAi(call) => call,
        AssistantCreateCall::Azure(_) => panic!("an azure call came back for an openai input"),
    }
}

fn azure_call(result: &AssistantCreateResult) -> &AzureCreateCall {
    match &result.call {
        AssistantCreateCall::Azure(call) => call,
        AssistantCreateCall::OpenAi(_) => panic!("an openai call came back for an azure input"),
    }
}

fn failure(result: AssistantCreateOutcome) -> AssistantCreateFailure {
    match result {
        Ok(created) => panic!("the call unexpectedly returned {}", created.assistant.id),
        Err(failure) => *failure,
    }
}

fn seen_auth(fixture: &Fixture, index: usize) -> String {
    fixture.transport.seen()[index].headers["authorization"].clone()
}

fn seen_deadline(fixture: &Fixture, index: usize) -> Duration {
    fixture.transport.seen()[index].deadline
}

async fn invoke(
    service: &AssistantRecords<'_, '_, Scripted>,
    request: &AssistantCreateInput,
    entry: Entry,
) -> AssistantCreateOutcome {
    match entry {
        Entry::Sync => service.create_assistants(request).await,
        Entry::Async => service.acreate_assistants(request).await,
    }
}

#[rstest]
#[case::sync(Entry::Sync)]
#[case::asynchronous(Entry::Async)]
#[tokio::test]
async fn assistant_create_optional_values(#[case] entry: Entry) {
    let fixture = Fixture::new(resource_sdk_settings::Settings::default());
    let redactor = redactor();
    let resolution = fixture.resolution();
    let service = fixture.service(&resolution, &redactor);
    let request = input(raw(r#"{
        "api_base": "https://request.test/v1",
        "api_key": "sk-request",
        "name": "",
        "description": "",
        "instructions": "",
        "tools": [],
        "tool_resources": {},
        "metadata": {},
        "temperature": 0,
        "top_p": 0,
        "response_format": {"type": "json_object"},
        "unknown_kwarg": "dropped",
        "stream": false
    }"#));
    let result = invoke(&service, &request, entry)
        .await
        .expect("the provider answered");
    let openai = openai_call(&result);
    assert_eq!(openai.is_async, entry == Entry::Async);
    assert_eq!(result.assistant.id, "asst_a");
    assert_eq!(result.assistant.object, "assistant");
    assert_eq!(result.assistant.model, "gpt-4o-mini");
    assert_eq!(result.assistant.created_at, 1);
    assert_eq!(result.assistant.name, None);
    assert!(result.assistant.tools.is_empty());
    let seen = fixture.transport.seen();
    assert_eq!(seen.len(), 1);
    let sent = &seen[0];
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.url, "https://request.test/v1/assistants");
    assert_eq!(sent.headers["authorization"], "Bearer sk-request");
    assert_eq!(sent.headers["openai-beta"], "assistants=v2");
    assert_eq!(sent.headers["accept"], "application/json");
    assert!(!sent.headers.contains_key("openai-organization"));
    assert_eq!(
        sent.body,
        create_body(raw(r#"{
            "model": "gpt-4o-mini",
            "name": "",
            "description": "",
            "instructions": "",
            "tools": [],
            "tool_resources": {},
            "metadata": {},
            "temperature": 0,
            "top_p": 0,
            "response_format": {"type": "json_object"}
        }"#))
    );

    // a null for every optional field stays out of the create data
    let null_request = input(raw(r#"{
        "api_base": "https://request.test/v1",
        "api_key": "sk-request",
        "name": null,
        "description": null,
        "instructions": null,
        "tools": null,
        "tool_resources": null,
        "metadata": null,
        "temperature": null,
        "top_p": null,
        "response_format": null
    }"#));
    let null_result = invoke(&service, &null_request, entry)
        .await
        .expect("the provider answered");
    assert_eq!(null_result.assistant.id, "asst_a");
    let all_seen = fixture.transport.seen();
    assert_eq!(all_seen.len(), 2);
    assert_eq!(
        all_seen[1].body,
        create_body(raw(r#"{"model": "gpt-4o-mini"}"#))
    );
}

// every layer of the credential chains set at once, so a test can see which one wins
fn layered_settings() -> resource_sdk_settings::Settings {
    let mut settings = bare_settings();
    settings.api_base = Some(String::from("https://global.test/v1"));
    settings.api_key = Some(SecretString::from("sk-global"));
    settings.organization = Some(String::from("org-global"));
    settings.openai_base_url = Some(String::from("https://base-url.test/v1"));
    settings.openai_api_key = Some(SecretString::from("sk-env"));
    settings.openai_organization = Some(String::from("org-env"));
    settings
}

#[tokio::test]
async fn assistant_create_credentials() {
    let redactor = redactor();
    let fixture = Fixture::new(layered_settings());
    let resolution = fixture.resolution();
    let service = fixture.service(&resolution, &redactor);

    // the request's base, key and organization win over the configured ones
    let request = input(raw(r#"{
        "api_base": "https://request.test/v1",
        "api_key": "sk-request",
        "organization": "org-request",
        "max_retries": 3,
        "client": "open_ai",
        "timeout": 7.5,
        "request_timeout": 12.0
    }"#));
    let result = service.create_assistants(&request).await.expect("answered");
    let call = openai_call(&result);
    assert_eq!(call.api_base, "https://request.test/v1");
    assert_eq!(call.max_retries, Some(3));
    assert_eq!(call.client, Some(ClientClass::OpenAi));
    assert_eq!(call.timeout, FineTuningTimeout::Seconds(7.5));
    assert_eq!(seen_auth(&fixture, 0), "Bearer sk-request");
    assert_eq!(
        fixture.transport.seen()[0].headers["openai-organization"],
        "org-request"
    );
    assert_eq!(seen_deadline(&fixture, 0), Duration::from_millis(7500));
}

#[tokio::test]
async fn assistant_create_configured_fallbacks() {
    let redactor = redactor();
    let fixture = Fixture::new(layered_settings());
    let resolution = fixture.resolution();
    let service = fixture.service(&resolution, &redactor);

    // the configured values fill in when the request leaves them out, and a zero falls through
    let fallback_request = input(raw(r#"{"timeout": 0, "request_timeout": 12.0}"#));
    let fallback_result = service
        .create_assistants(&fallback_request)
        .await
        .expect("answered");
    assert_eq!(
        openai_call(&fallback_result).api_base,
        "https://global.test/v1"
    );
    assert_eq!(seen_auth(&fixture, 0), "Bearer sk-global");
    assert_eq!(
        fixture.transport.seen()[0].headers["openai-organization"],
        "org-global"
    );
    assert_eq!(seen_deadline(&fixture, 0), Duration::from_secs(12));
}

#[tokio::test]
async fn assistant_create_openai_env_chain() {
    let redactor = redactor();
    // an empty base falls through, the key chain reaches the env name, and a numeric text converts
    let mut settings = bare_settings();
    settings.api_base = Some(String::new());
    settings.openai_key = Some(SecretString::from("sk-openai-key"));
    settings.openai_base_url = Some(String::from("https://base-url.test/v1"));
    let fixture = Fixture::new(settings);
    let resolution = fixture.resolution();
    let service = fixture.service(&resolution, &redactor);
    let text_request = input(raw(r#"{"timeout": "7.5"}"#));
    let text_result = service
        .create_assistants(&text_request)
        .await
        .expect("answered");
    assert_eq!(
        openai_call(&text_result).api_base,
        "https://base-url.test/v1"
    );
    assert_eq!(seen_auth(&fixture, 0), "Bearer sk-openai-key");
    assert_eq!(seen_deadline(&fixture, 0), Duration::from_millis(7500));
}

#[tokio::test]
async fn assistant_create_default_base_and_missing_key() {
    let redactor = redactor();
    // with no key the SDK raises its own OpenAIError
    let fixture_default = Fixture::new(resource_sdk_settings::Settings::default());
    let default_resolution = fixture_default.resolution();
    let default_service = fixture_default.service(&default_resolution, &redactor);
    let missing = failure(default_service.create_assistants(&input(raw("{}"))).await);
    assert_eq!(missing.class, "OpenAIError");
    assert_eq!(missing.status, None);

    // a missing base is the OpenAI default, unlike Azure, which has none
    let fixture_base = Fixture::new(openai_settings("sk-test"));
    let base_resolution = fixture_base.resolution();
    let base_service = fixture_base.service(&base_resolution, &redactor);
    let base_result = base_service
        .create_assistants(&input(raw("{}")))
        .await
        .expect("answered");
    assert_eq!(
        openai_call(&base_result).api_base,
        "https://api.openai.com/v1"
    );
    assert_eq!(
        fixture_base.transport.seen()[0].url,
        "https://api.openai.com/v1/assistants"
    );
}

#[tokio::test]
async fn assistant_create_structured_timeout() {
    let redactor = redactor();
    let fixture = Fixture::new(openai_settings("sk-test"));
    let resolution = fixture.resolution();
    let service = fixture.service(&resolution, &redactor);

    // a structured timeout survives for openai and bounds the call by its read phase
    let phases_request = input(raw(r#"{"timeout": {"read": 9.0, "connect": 2.0}}"#));
    service
        .create_assistants(&phases_request)
        .await
        .expect("answered");
    assert_eq!(seen_deadline(&fixture, 0), Duration::from_secs(9));

    // an empty text timeout falls through like an absent one
    let empty_text_request = input(raw(r#"{"timeout": "", "request_timeout": 12.0}"#));
    service
        .create_assistants(&empty_text_request)
        .await
        .expect("answered");
    assert_eq!(seen_deadline(&fixture, 1), Duration::from_secs(12));

    // a phase map whose read is zero keeps the SDK default deadline
    let zero_read_request = input(raw(r#"{"timeout": {"read": 0.0}}"#));
    service
        .create_assistants(&zero_read_request)
        .await
        .expect("answered");
    assert_eq!(seen_deadline(&fixture, 2), Duration::from_secs(600));
}

#[rstest]
#[case::openai_client(ClientClass::OpenAi, None)]
#[case::azure_openai_client(ClientClass::AzureOpenAi, None)]
#[case::async_openai_client(ClientClass::AsyncOpenAi, Some(ClientClass::AsyncOpenAi))]
#[case::async_azure_client(ClientClass::AsyncAzureOpenAi, Some(ClientClass::AsyncAzureOpenAi))]
#[tokio::test]
async fn assistant_create_azure_options(
    #[case] client: ClientClass,
    #[case] expected: Option<ClientClass>,
) {
    let redactor = redactor();
    let mut settings = bare_settings();
    settings.azure_api_base = Some(String::from("https://azure.test"));
    settings.azure_api_version = Some(String::from("2024-02-15-preview"));
    settings.azure_api_key = Some(SecretString::from("key-settings"));
    let fixture = Fixture::new(settings);
    let resolution = fixture.resolution();
    let service = fixture.service(&resolution, &redactor);
    let mut request = input(raw(r#"{"provider": "azure"}"#));
    request.client = Some(client);
    let result = service.create_assistants(&request).await.expect("answered");
    assert_eq!(azure_call(&result).client, expected);
}

#[tokio::test]
async fn assistant_create_azure_request_values() {
    let redactor = redactor();
    let mut settings = bare_settings();
    settings.azure_api_base = Some(String::from("https://azure.test"));
    settings.azure_api_version = Some(String::from("2024-02-15-preview"));
    settings.azure_api_key = Some(SecretString::from("key-settings"));
    let fixture = Fixture::new(settings);
    let resolution = fixture.resolution();
    let service = fixture.service(&resolution, &redactor);

    // the request's api_version wins over the configured one, and a numeric timeout bounds the call
    let request = input(raw(r#"{
        "provider": "azure",
        "api_version": "2020-01-01",
        "timeout": 7.5
    }"#));
    let result = service.create_assistants(&request).await.expect("answered");
    let call = azure_call(&result);
    assert_eq!(call.api_version.as_deref(), Some("2020-01-01"));
    assert_eq!(call.litellm_params.timeout, Some(7.5_f64));
    let seen = fixture.transport.seen();
    assert_eq!(
        seen[0].url,
        "https://azure.test/openai/assistants?api-version=2020-01-01"
    );
    assert_eq!(seen[0].deadline, Duration::from_millis(7500));

    // a structured timeout is accepted for azure and leaves the deadline at the SDK default
    let phases_request = input(raw(r#"{"provider": "azure", "timeout": {"read": 9.0}}"#));
    let phases_result = service
        .create_assistants(&phases_request)
        .await
        .expect("answered");
    assert_eq!(azure_call(&phases_result).litellm_params.timeout, None);
    assert_eq!(
        fixture.transport.seen()[1].deadline,
        Duration::from_secs(600)
    );

    // an azure call with no base anywhere has no default and fails as a bare ValueError
    let fixture_unset = Fixture::new(resource_sdk_settings::Settings::default());
    let unset_resolution = fixture_unset.resolution();
    let unset_service = fixture_unset.service(&unset_resolution, &redactor);
    let unset_request = input(raw(r#"{"provider": "azure", "api_key": "sk-azure"}"#));
    let missing = failure(unset_service.create_assistants(&unset_request).await);
    assert_eq!(missing.class, "ValueError");
    assert_eq!(missing.status, None);
}

#[rstest]
#[case::absent(false)]
#[case::empty(true)]
#[tokio::test]
async fn assistant_create_azure_token(#[case] empty_extra_body: bool) {
    let redactor = redactor();
    let mut settings = bare_settings();
    settings.azure_api_base = Some(String::from("https://azure.test"));
    settings.azure_api_version = Some(String::from("2024-02-15-preview"));
    settings.azure_openai_api_key = Some(SecretString::from("key-settings"));
    let fixture = Fixture::new(settings).env("AZURE_AD_TOKEN", "token-env");
    let resolution = fixture.resolution();
    let service = fixture.service(&resolution, &redactor);

    // an explicit null extra_body reads the environment token, which the facade drops
    let request = input(raw(r#"{"provider": "azure", "extra_body": null}"#));
    let result = service
        .acreate_assistants(&request)
        .await
        .expect("answered");
    let call = azure_call(&result);
    assert_eq!(
        call.azure_ad_token
            .as_ref()
            .map(SecretString::expose_secret),
        Some("token-env")
    );
    assert_eq!(call.api_base.as_deref(), Some("https://azure.test"));
    assert_eq!(call.api_version.as_deref(), Some("2024-02-15-preview"));
    let seen = fixture.transport.seen();
    assert_eq!(
        seen[0].url,
        "https://azure.test/openai/assistants?api-version=2024-02-15-preview"
    );
    assert_eq!(seen[0].headers["api-key"], "key-settings");
    assert!(!seen[0].headers.contains_key("authorization"));

    // an absent or empty extra_body reads no environment token
    let mut plain_request = input(raw(r#"{"provider": "azure"}"#));
    if empty_extra_body {
        plain_request.extra_body = Some(Some(BTreeMap::new()));
    }
    let plain_result = service
        .create_assistants(&plain_request)
        .await
        .expect("answered");
    assert!(azure_call(&plain_result).azure_ad_token.is_none());

    // a body token wins and is popped; every other extra_body key stays out of the request
    let body_request = input(raw(r#"{
        "provider": "azure",
        "azure_ad_token": "token-params",
        "extra_body": {"azure_ad_token": "token-body", "other": "x"}
    }"#));
    let body_result = service
        .acreate_assistants(&body_request)
        .await
        .expect("answered");
    let body_call = azure_call(&body_result);
    assert_eq!(
        body_call
            .azure_ad_token
            .as_ref()
            .map(SecretString::expose_secret),
        Some("token-body")
    );
    assert_eq!(
        body_call
            .litellm_params
            .azure_ad_token
            .as_ref()
            .map(SecretString::expose_secret),
        Some("token-params")
    );
    assert_eq!(
        fixture.transport.seen()[2].body,
        create_body(raw(r#"{"model": "gpt-4o-mini"}"#))
    );

    // the litellm_params token reaches the SDK and the facade's own token never does
    let mut bearer_settings = bare_settings();
    bearer_settings.azure_api_base = Some(String::from("https://azure.test"));
    let fixture_bearer = Fixture::new(bearer_settings);
    let bearer_resolution = fixture_bearer.resolution();
    let bearer_service = fixture_bearer.service(&bearer_resolution, &redactor);
    let params_request = input(raw(
        r#"{"provider": "azure", "azure_ad_token": "token-params"}"#,
    ));
    bearer_service
        .create_assistants(&params_request)
        .await
        .expect("answered");
    let bearer_seen = fixture_bearer.transport.seen();
    assert_eq!(
        bearer_seen[0].headers["authorization"],
        "Bearer token-params"
    );
    assert!(!bearer_seen[0].headers.contains_key("api-key"));
}

#[rstest]
#[case::sync_absent(Entry::Sync, None)]
#[case::sync_false(Entry::Sync, Some(false))]
#[case::sync_true(Entry::Sync, Some(true))]
#[case::async_absent(Entry::Async, None)]
#[case::async_false(Entry::Async, Some(false))]
#[case::async_true(Entry::Async, Some(true))]
#[tokio::test]
async fn assistant_create_flag_and_failures(#[case] entry: Entry, #[case] flag: Option<bool>) {
    let redactor = redactor();
    let fixture = Fixture::new(openai_settings("sk-test"));
    let resolution = fixture.resolution();
    let service = fixture.service(&resolution, &redactor);

    // any bool, or none at all, is a legal flag; the async entry forces it true whatever the caller said
    let mut request = input(raw("{}"));
    request.async_create_assistants = flag.map(AsyncFlag::Bool);
    let result = invoke(&service, &request, entry).await.expect("answered");
    let expected = entry == Entry::Async || flag == Some(true);
    assert_eq!(openai_call(&result).is_async, expected);

    // anything else raises the ValueError before a client is built
    let mut int_flag_request = input(raw("{}"));
    int_flag_request.async_create_assistants =
        Some(serde_json::from_str::<AsyncFlag>("1").expect("a flag"));
    let value_error = failure(service.create_assistants(&int_flag_request).await);
    assert_eq!(value_error.class, "ValueError");
    assert_eq!(value_error.status, None);
    assert_eq!(
        value_error.message,
        "Invalid value passed in for async_create_assistants. Only bool or None allowed"
    );
    let mut text_flag_request = input(raw("{}"));
    text_flag_request.async_create_assistants =
        Some(serde_json::from_str::<AsyncFlag>("\"true\"").expect("a flag"));
    let text_value_error = failure(service.create_assistants(&text_flag_request).await);
    assert_eq!(text_value_error.class, "ValueError");
    assert_eq!(
        text_value_error.message,
        "Invalid value passed in for async_create_assistants. Only bool or None allowed"
    );

    // an unsupported provider is a 400 before any request is sent
    let unsupported_request = input(raw(r#"{"provider": "anthropic", "model": "claude-3"}"#));
    let unsupported = failure(service.create_assistants(&unsupported_request).await);
    assert_eq!(unsupported.class, "BadRequestError");
    assert_eq!(unsupported.status, Some(400));
    assert_eq!(unsupported.kind, WireField::Null);
    assert_eq!(unsupported.param, WireField::Null);
    assert_eq!(unsupported.code, WireField::Null);
    assert_eq!(
        unsupported.message,
        "LiteLLM doesn't support anthropic for 'create_assistants'. Only 'openai' is supported."
    );

    // a provider that answers nothing is the wrapper's own 500
    let absent = no_response();
    assert_eq!(absent.class, "InternalServerError");
    assert_eq!(absent.status, Some(500));
    assert_eq!(
        absent.kind,
        WireField::Text(String::from("internal_server_error"))
    );
    assert_eq!(absent.param, WireField::Null);
    assert_eq!(absent.code, WireField::Null);
    assert_eq!(
        absent.message,
        "No response returned from 'create_assistants'"
    );

    // the sync entry keeps the body's type/param/code; the async one maps through exception_type to nulls
    let body_404 = r#"{"error":{"message":"no such assistant","type":"invalid_request_error","param":"assistant_id","code":"missing"}}"#;
    let fixture_404 = Fixture::new(openai_settings("sk-test"))
        .reply(404, body_404)
        .reply(404, body_404);
    let not_found_resolution = fixture_404.resolution();
    let not_found_service = fixture_404.service(&not_found_resolution, &redactor);
    let not_found_request = input(raw("{}"));
    let sync = failure(
        not_found_service
            .create_assistants(&not_found_request)
            .await,
    );
    assert_eq!(sync.class, "NotFoundError");
    assert_eq!(sync.status, Some(404));
    assert_eq!(
        sync.kind,
        WireField::Text(String::from("invalid_request_error"))
    );
    assert_eq!(sync.param, WireField::Text(String::from("assistant_id")));
    assert_eq!(sync.code, WireField::Text(String::from("missing")));
    let asynchronous = failure(
        not_found_service
            .acreate_assistants(&not_found_request)
            .await,
    );
    assert_eq!(asynchronous.class, "NotFoundError");
    assert_eq!(asynchronous.status, Some(404));
    assert_eq!(asynchronous.kind, WireField::Null);
    assert_eq!(asynchronous.param, WireField::Null);
    assert_eq!(asynchronous.code, WireField::Null);

    // a 500 keeps the body's type in sync and uses the internal_server_error literal in async
    let body_500 = r#"{"error":{"message":"boom","type":"server_error","param":null,"code":null}}"#;
    let fixture_500 = Fixture::new(openai_settings("sk-test"))
        .reply(500, body_500)
        .reply(500, body_500);
    let server_error_resolution = fixture_500.resolution();
    let server_error_service = fixture_500.service(&server_error_resolution, &redactor);
    let server_error_request = input(raw("{}"));
    let sync_500 = failure(
        server_error_service
            .create_assistants(&server_error_request)
            .await,
    );
    assert_eq!(sync_500.class, "InternalServerError");
    assert_eq!(sync_500.status, Some(500));
    assert_eq!(sync_500.kind, WireField::Text(String::from("server_error")));
    let async_500 = failure(
        server_error_service
            .acreate_assistants(&server_error_request)
            .await,
    );
    assert_eq!(async_500.class, "InternalServerError");
    assert_eq!(async_500.status, Some(500));
    assert_eq!(
        async_500.kind,
        WireField::Text(String::from("internal_server_error"))
    );
    assert_eq!(async_500.param, WireField::Null);
    assert_eq!(async_500.code, WireField::Null);

    // a 400 keeps the body's type/param/code in both entries: exception_type forwards the bad-request body whole
    let body_400 = r#"{"error":{"message":"bad request","type":"invalid_request_error","param":"model","code":"bad"}}"#;
    let fixture_400 = Fixture::new(openai_settings("sk-test"))
        .reply(400, body_400)
        .reply(400, body_400);
    let bad_request_resolution = fixture_400.resolution();
    let bad_request_service = fixture_400.service(&bad_request_resolution, &redactor);
    let bad_request_request = input(raw("{}"));
    let sync_400 = failure(
        bad_request_service
            .create_assistants(&bad_request_request)
            .await,
    );
    assert_eq!(sync_400.class, "BadRequestError");
    assert_eq!(sync_400.status, Some(400));
    let async_400 = failure(
        bad_request_service
            .acreate_assistants(&bad_request_request)
            .await,
    );
    assert_eq!(async_400.class, "BadRequestError");
    assert_eq!(async_400.status, Some(400));
    assert_eq!(
        async_400.kind,
        WireField::Text(String::from("invalid_request_error"))
    );
    assert_eq!(async_400.param, WireField::Text(String::from("model")));
    assert_eq!(async_400.code, WireField::Text(String::from("bad")));

    // a raw failure with no status is an APIConnection 500 in the async entry and a bare ValueError in sync
    let async_raised = raised(
        &Raised::Local(String::from("no provider")),
        "gpt-4o-mini",
        "openai",
        Entry::Async,
        &redactor,
    );
    assert_eq!(async_raised.class, "APIConnectionError");
    assert_eq!(async_raised.status, Some(500));
    let sync_raised = raised(
        &Raised::Local(String::from("no provider")),
        "gpt-4o-mini",
        "openai",
        Entry::Sync,
        &redactor,
    );
    assert_eq!(sync_raised.class, "ValueError");
    assert_eq!(sync_raised.status, None);
}

#[rstest]
#[case::bad_request(400, "BadRequestError")]
#[case::unauthorized(401, "AuthenticationError")]
#[case::forbidden(403, "PermissionDeniedError")]
#[case::conflict(409, "ConflictError")]
#[case::unprocessable(422, "UnprocessableEntityError")]
#[case::rate_limit(429, "RateLimitError")]
#[tokio::test]
async fn assistant_create_status_classes(#[case] status: u16, #[case] class: &str) {
    let redactor = redactor();
    let body = r#"{"error":{"message":"rejected","type":"invalid_request_error","param":"model","code":"bad"}}"#;
    let fixture = Fixture::new(openai_settings("sk-test")).reply(status, body);
    let resolution = fixture.resolution();
    let service = fixture.service(&resolution, &redactor);
    let sync = failure(service.create_assistants(&input(raw("{}"))).await);
    assert_eq!(sync.class, class);
    assert_eq!(sync.status, Some(status));
}

#[tokio::test]
async fn assistant_create_status_message() {
    let redactor = redactor();
    let body = r#"{"error":{"message":"no such assistant","type":"invalid_request_error"}}"#;
    let fixture = Fixture::new(openai_settings("sk-test")).reply(404, body);
    let resolution = fixture.resolution();
    let service = fixture.service(&resolution, &redactor);
    let sync = failure(service.create_assistants(&input(raw("{}"))).await);
    assert_eq!(sync.message, format!("Error code: 404 - {body}"));

    // a body that is not UTF-8 decodes like httpx's errors="replace": one U+FFFD per bad sequence
    let fixture_bad = Fixture::new(openai_settings("sk-test"));
    fixture_bad
        .transport
        .reply_bytes(500, b"oops \xff".to_vec());
    let bad_resolution = fixture_bad.resolution();
    let bad_service = fixture_bad.service(&bad_resolution, &redactor);
    let bad_sync = failure(bad_service.create_assistants(&input(raw("{}"))).await);
    assert_eq!(bad_sync.message, "Error code: 500 - oops \u{fffd}");
}

#[test]
fn assistant_create_retry_values() {
    assert_eq!(super::projection::retries(Some(3)).ok(), Some(Some(3)));
    assert_eq!(super::projection::retries(None).ok(), Some(None));
    assert_eq!(
        super::projection::retries(Some(-1))
            .unwrap_err()
            .to_string(),
        "max_retries must be a non-negative integer, got -1"
    );
}

#[test]
fn assistant_create_timeout_values_parse() {
    let number: CreateTimeout = serde_json::from_str("0").expect("a number");
    assert_eq!(number, CreateTimeout::Seconds(0.0));
    let arguments: CreateTimeout = serde_json::from_str("\"7.5\"").expect("text");
    assert_eq!(arguments, CreateTimeout::Text(String::from("7.5")));
    let phases: CreateTimeout = serde_json::from_str(r#"{"read": 9.0}"#).expect("phases");
    match phases {
        CreateTimeout::Phases(given) => assert_eq!(given.read, Some(9.0_f64)),
        CreateTimeout::Seconds(_) | CreateTimeout::Text(_) => panic!("a phase map parsed as plain"),
    }
}
