#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_azure::credentials::source::CredentialSources;
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::sections::azure_endpoints::Settings as AzureSettings;
use betterlite_config::sections::resource_sdk_settings::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::assistants::error::AssistantsError;
use betterlite_openai::assistants::records::{AssistantDeleted, AssistantPage};
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_prompts::arguments::json::Json;
use http::HeaderMap;
use secrecy::{ExposeSecret, SecretString};

use super::call::{AssistantCall, AssistantProviderCall, AzureAssistantCall};
use super::error::AssistantReadDeleteError;
use super::request::{
    AssistantOperation, AssistantParams, AssistantReadDeleteInput, AssistantTimeout,
    AssistantWrapper, SuppliedClient, SuppliedClientKind, TimeoutPhases,
};
use super::sdk_dispatch::{AssistantSdkError, NoSources, SdkDispatch};
use super::{
    AssistantReadDelete, AssistantReadDeleteDispatch, AssistantReadDeleteResult, AssistantReply,
    AssistantStep,
};
use crate::api::api_boundaries::ApiLimits;
use crate::api::resource_sdk_settings::timeout::{FineTuningTimeout, StructuredTimeout};

const PAGE: &str = r#"{"object":"list","data":[{"id":"asst_b","object":"assistant","created_at":1,"model":"gpt-4o","tools":[]}],"has_more":true,"first_id":"asst_b","last_id":"asst_b"}"#;
const DELETED: &str = r#"{"id":"asst_a","object":"assistant.deleted","deleted":true}"#;
const NOT_FOUND_BODY: &str = r#"{"error":{"message":"missing","type":"invalid_request_error","param":"assistant_id","code":"missing"}}"#;
const OPENAI_BASE: &str = "https://openai.test/v1";
const AZURE_BASE: &str = "https://azure.test/v1";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    method: String,
    url: String,
    authorization: Option<String>,
    api_key: Option<String>,
    organization: Option<String>,
    beta: Option<String>,
}

struct Recorder {
    seen: Mutex<Vec<Seen>>,
    replies: Mutex<VecDeque<(u16, String)>>,
}

impl AssistantsTransport for Recorder {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let mut url = request.url.clone();
        merge_query(&mut url, &request.query);
        let header = |name: &str| {
            request
                .headers
                .get(name)
                .map(|value| value.to_str().unwrap_or_default().to_owned())
        };
        self.seen.lock().unwrap().push(Seen {
            method: request.method.as_str().to_owned(),
            url: url.as_str().to_owned(),
            authorization: header("authorization"),
            api_key: header("api-key"),
            organization: header("openai-organization"),
            beta: header("openai-beta"),
        });
        let (status, text) = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| (500, String::new()));
        ready(Ok(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: text.into_bytes(),
        }))
    }
}

impl Recorder {
    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

type Sdk = SdkDispatch<Recorder, NoSources>;

fn harness(status: u16, body: &str) -> (Arc<Recorder>, Sdk) {
    let recorder = Arc::new(Recorder {
        seen: Mutex::new(Vec::new()),
        replies: Mutex::new(VecDeque::from([(status, body.to_owned())])),
    });
    let dispatch = SdkDispatch {
        transport: Arc::clone(&recorder),
        sources: NoSources,
        environment: BTreeMap::new(),
        azure: AzureSettings::default(),
    };
    (recorder, dispatch)
}

fn facade<Dispatch>(dispatch: Dispatch, settings: Settings) -> AssistantReadDelete<Dispatch> {
    AssistantReadDelete {
        dispatch,
        limits: ApiLimits::default(),
        settings,
    }
}

fn input_of(operation: AssistantOperation, provider: &str) -> AssistantReadDeleteInput {
    AssistantReadDeleteInput {
        operation,
        provider: provider.to_owned(),
        assistant_id: None,
        entry: AssistantWrapper::Sync,
        aget_assistants: None,
        async_delete_assistants: None,
        extra_body: None,
        client: None,
        after: None,
        before: None,
        limit: None,
        order: None,
        params: AssistantParams::default(),
    }
}

fn list_openai(entry: AssistantWrapper) -> AssistantReadDeleteInput {
    let mut input = input_of(AssistantOperation::List, "openai");
    input.entry = entry;
    input.params.api_base = Some(OPENAI_BASE.to_owned());
    input.params.api_key = Some(SecretString::from("sk-request"));
    input
}

fn delete_azure(entry: AssistantWrapper) -> AssistantReadDeleteInput {
    let mut input = input_of(AssistantOperation::Delete, "azure");
    input.entry = entry;
    input.assistant_id = Some("asst_a".to_owned());
    input.params.api_base = Some(AZURE_BASE.to_owned());
    input.params.api_key = Some(SecretString::from("sk-request"));
    input.params.api_version = Some("2024-02-01".to_owned());
    input
}

fn supplied(kind: SuppliedClientKind, api_base: &str) -> SuppliedClient {
    SuppliedClient {
        kind,
        api_base: Some(api_base.to_owned()),
        api_key: Some(SecretString::from("sk-supplied")),
        api_version: Some("2024-02-01".to_owned()),
    }
}

fn page_fixture() -> AssistantPage {
    serde_json::from_str(PAGE).unwrap()
}

fn assert_page(step: &AssistantStep) {
    let AssistantStep::Page(page) = step else {
        panic!("expected a page, got {step:?}");
    };
    assert_eq!(page, &page_fixture());
    assert_eq!(page.data[0].id, "asst_b");
    assert_eq!(page.has_more, Some(true));
}

fn assert_azure_delete(seen: &Seen, step: &AssistantStep) {
    assert_eq!(seen.method, "DELETE");
    assert_eq!(
        seen.url,
        "https://azure.test/v1/openai/assistants/asst_a?api-version=2024-02-01"
    );
    let AssistantStep::Deleted(deleted) = step else {
        panic!("expected a deleted object, got {step:?}");
    };
    let expected: AssistantDeleted = serde_json::from_str(DELETED).unwrap();
    assert_eq!(deleted, &expected);
}

async fn recorded(input: &AssistantReadDeleteInput) -> (Vec<Seen>, AssistantReadDeleteResult) {
    let (recorder, dispatch) = harness(200, PAGE);
    let result = facade(dispatch, Settings::default())
        .run(input)
        .await
        .unwrap();
    (recorder.requests(), result)
}

fn openai_timeout(result: &AssistantReadDeleteResult) -> Option<Duration> {
    let AssistantProviderCall::OpenAi(options) = &result.call.provider else {
        panic!("expected the openai branch");
    };
    options.timeout
}

fn openai_retries(result: &AssistantReadDeleteResult) -> Option<u32> {
    let AssistantProviderCall::OpenAi(options) = &result.call.provider else {
        panic!("expected the openai branch");
    };
    options.max_retries
}

fn azure_values(result: &AssistantReadDeleteResult) -> &AzureAssistantCall {
    let AssistantProviderCall::Azure(values) = &result.call.provider else {
        panic!("expected the azure branch");
    };
    values
}

fn exposed(secret: Option<&SecretString>) -> Option<&str> {
    secret.map(ExposeSecret::expose_secret)
}

#[tokio::test]
async fn list_drops_pagination_and_sends_the_sdk_defaults() {
    let mut input = list_openai(AssistantWrapper::Async);
    input.after = Some("asst_a".to_owned());
    input.before = Some("asst_z".to_owned());
    input.limit = Some(3);
    input.order = Some("asc".to_owned());
    let (requests, result) = recorded(&input).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert!(
        requests[0]
            .url
            .ends_with("/v1/assistants?limit=20&order=desc")
    );
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer sk-request")
    );
    assert_eq!(requests[0].beta.as_deref(), Some("assistants=v2"));
    assert_page(&result.step);
}

#[tokio::test]
async fn sync_list_without_a_flag_makes_the_same_request() {
    let input = list_openai(AssistantWrapper::Sync);
    let (requests, result) = recorded(&input).await;
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .url
            .ends_with("/v1/assistants?limit=20&order=desc")
    );
    assert_page(&result.step);
}

#[tokio::test]
async fn sync_list_with_the_async_flag_hands_back_a_coroutine() {
    let (recorder, dispatch) = harness(200, PAGE);
    let mut input = list_openai(AssistantWrapper::Sync);
    input.aget_assistants = Some(Json::Bool(true));
    let result = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_eq!(result.step, AssistantStep::Coroutine);
    assert!(recorder.requests().is_empty());
}

#[tokio::test]
async fn azure_list_keeps_api_version_and_drops_pagination() {
    let mut input = delete_azure(AssistantWrapper::Async);
    input.operation = AssistantOperation::List;
    input.assistant_id = None;
    input.after = Some("asst_a".to_owned());
    input.order = Some("asc".to_owned());
    let (requests, result) = recorded(&input).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url,
        "https://azure.test/v1/openai/assistants?api-version=2024-02-01"
    );
    assert_eq!(requests[0].api_key.as_deref(), Some("sk-request"));
    assert_eq!(requests[0].beta.as_deref(), Some("assistants=v2"));
    assert_page(&result.step);
}

async fn delete_in_the_sync_entry() {
    let input = delete_azure(AssistantWrapper::Sync);
    let (recorder, dispatch) = harness(200, DELETED);
    let result = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_azure_delete(&recorder.requests()[0], &result.step);
}

async fn delete_in_the_async_entry() {
    let input = delete_azure(AssistantWrapper::Async);
    let (recorder, dispatch) = harness(200, DELETED);
    let result = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_azure_delete(&recorder.requests()[0], &result.step);
}

#[tokio::test]
async fn delete_sends_the_identifier_in_both_entries() {
    delete_in_the_sync_entry().await;
    delete_in_the_async_entry().await;
}

async fn azure_delete_discards(kind: SuppliedClientKind) {
    let (recorder, dispatch) = harness(200, DELETED);
    let mut input = delete_azure(AssistantWrapper::Async);
    input.client = Some(supplied(kind, AZURE_BASE));
    facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    let requests = recorder.requests();
    assert_eq!(requests[0].api_key.as_deref(), Some("sk-request"));
    assert!(requests[0].authorization.is_none());
}

#[tokio::test]
async fn azure_delete_discards_openai_class_clients_and_keeps_async_ones() {
    azure_delete_discards(SuppliedClientKind::OpenAi).await;
    azure_delete_discards(SuppliedClientKind::AzureOpenAi).await;

    let (recorder, dispatch) = harness(200, DELETED);
    let mut input = delete_azure(AssistantWrapper::Sync);
    input.client = Some(supplied(SuppliedClientKind::AsyncOpenAi, AZURE_BASE));
    let result = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_eq!(result.step, AssistantStep::Coroutine);
    assert!(recorder.requests().is_empty());
}

async fn azure_list_keeps_an_openai_client() {
    let (recorder, dispatch) = harness(200, PAGE);
    let mut input = delete_azure(AssistantWrapper::Async);
    input.operation = AssistantOperation::List;
    input.assistant_id = None;
    input.client = Some(supplied(SuppliedClientKind::OpenAi, OPENAI_BASE));
    facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    let requests = recorder.requests();
    assert_eq!(
        requests[0].url,
        format!("{OPENAI_BASE}/assistants?limit=20&order=desc")
    );
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer sk-supplied")
    );
    assert!(requests[0].api_key.is_none());
}

async fn azure_list_keeps_an_azure_client() {
    let (recorder, dispatch) = harness(200, PAGE);
    let mut input = delete_azure(AssistantWrapper::Async);
    input.operation = AssistantOperation::List;
    input.assistant_id = None;
    input.client = Some(supplied(SuppliedClientKind::AzureOpenAi, AZURE_BASE));
    facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    let requests = recorder.requests();
    assert_eq!(
        requests[0].url, "https://azure.test/v1/openai/assistants?api-version=2024-02-01",
        "an Azure client keeps the Azure request path"
    );
    assert_eq!(requests[0].api_key.as_deref(), Some("sk-supplied"));
    assert!(requests[0].authorization.is_none());
}

async fn azure_delete_keeps_an_async_openai_client() {
    let (recorder, dispatch) = harness(200, DELETED);
    let mut input = delete_azure(AssistantWrapper::Async);
    input.client = Some(supplied(SuppliedClientKind::AsyncOpenAi, OPENAI_BASE));
    facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    let requests = recorder.requests();
    assert_eq!(
        requests[0].url,
        format!("{OPENAI_BASE}/assistants/asst_a"),
        "an async OpenAI client survives the Azure delete and sends the OpenAI request"
    );
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer sk-supplied")
    );
}

async fn openai_delete_keeps_an_openai_client() {
    let (recorder, dispatch) = harness(200, DELETED);
    let mut input = list_openai(AssistantWrapper::Sync);
    input.operation = AssistantOperation::Delete;
    input.assistant_id = Some("asst_a".to_owned());
    input.client = Some(supplied(SuppliedClientKind::OpenAi, OPENAI_BASE));
    facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    let requests = recorder.requests();
    assert_eq!(requests[0].url, format!("{OPENAI_BASE}/assistants/asst_a"));
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer sk-supplied")
    );
}

#[tokio::test]
async fn azure_list_and_openai_delete_keep_openai_class_clients() {
    azure_list_keeps_an_openai_client().await;
    azure_list_keeps_an_azure_client().await;
    azure_delete_keeps_an_async_openai_client().await;
    openai_delete_keeps_an_openai_client().await;
}

fn keyed_settings() -> Settings {
    Settings {
        api_base: Some("https://settings.test/v1".to_owned()),
        api_key: Some(SecretString::from("sk-settings")),
        openai_base_url: Some("https://env.test/v1".to_owned()),
        openai_api_key: Some(SecretString::from("sk-env")),
        organization: Some("org-settings".to_owned()),
        openai_organization: Some("org-env".to_owned()),
        ..Settings::default()
    }
}

async fn settings_win_when_the_request_is_empty() {
    let (recorder, dispatch) = harness(200, PAGE);
    let mut input = list_openai(AssistantWrapper::Sync);
    input.params.api_base = Some(String::new());
    input.params.api_key = None;
    facade(dispatch, keyed_settings())
        .run(&input)
        .await
        .unwrap();
    let requests = recorder.requests();
    assert_eq!(
        requests[0].url,
        "https://settings.test/v1/assistants?limit=20&order=desc"
    );
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer sk-settings")
    );
    assert_eq!(requests[0].organization.as_deref(), Some("org-settings"));
}

async fn env_wins_when_settings_are_empty() {
    let empty = Settings {
        api_base: Some(String::new()),
        api_key: Some(SecretString::from("")),
        organization: Some(String::new()),
        ..keyed_settings()
    };
    let (recorder, dispatch) = harness(200, PAGE);
    let mut input = list_openai(AssistantWrapper::Sync);
    input.params.api_base = None;
    input.params.api_key = Some(SecretString::from(""));
    facade(dispatch, empty).run(&input).await.unwrap();
    let requests = recorder.requests();
    assert_eq!(
        requests[0].url,
        "https://env.test/v1/assistants?limit=20&order=desc"
    );
    assert_eq!(requests[0].authorization.as_deref(), Some("Bearer sk-env"));
    assert_eq!(requests[0].organization.as_deref(), Some("org-env"));
}

async fn the_default_base_serves_an_unconfigured_call() {
    let (recorder, dispatch) = harness(200, PAGE);
    let mut input = list_openai(AssistantWrapper::Sync);
    input.params.api_base = None;
    input.params.api_key = None;
    facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    let requests = recorder.requests();
    assert_eq!(
        requests[0].url,
        "https://api.openai.com/v1/assistants?limit=20&order=desc"
    );
    assert!(requests[0].authorization.is_none());
}

#[tokio::test]
async fn openai_credentials_follow_the_litellm_chain() {
    settings_win_when_the_request_is_empty().await;
    env_wins_when_settings_are_empty().await;
    the_default_base_serves_an_unconfigured_call().await;
}

fn timeout_input(
    timeout: AssistantTimeout,
    request_timeout: Option<f64>,
) -> AssistantReadDeleteInput {
    let mut input = list_openai(AssistantWrapper::Sync);
    input.params.timeout = Some(timeout);
    input.params.request_timeout = request_timeout;
    input
}

async fn the_default_timeout_is_ten_minutes() {
    let (_, default_result) = recorded(&list_openai(AssistantWrapper::Sync)).await;
    assert_eq!(
        default_result.call.timeout,
        FineTuningTimeout::Seconds(600.0)
    );
    assert_eq!(
        openai_timeout(&default_result),
        Some(Duration::from_secs(600)),
        "the OpenAI SDK gets the resolved default"
    );
}

async fn a_number_wins_over_everything() {
    let input = timeout_input(AssistantTimeout::Number(9.5), None);
    let (_, result) = recorded(&input).await;
    assert_eq!(result.call.timeout, FineTuningTimeout::Seconds(9.5));
}

async fn a_zero_timeout_falls_to_the_request_timeout() {
    let input = timeout_input(AssistantTimeout::Number(0.0), Some(8.0));
    let (_, result) = recorded(&input).await;
    assert_eq!(result.call.timeout, FineTuningTimeout::Seconds(8.0));
}

async fn an_empty_text_timeout_falls_to_the_request_timeout() {
    let input = timeout_input(AssistantTimeout::Text(String::new()), Some(8.0));
    let (_, result) = recorded(&input).await;
    assert_eq!(result.call.timeout, FineTuningTimeout::Seconds(8.0));
}

async fn a_text_timeout_is_parsed_like_float() {
    let input = timeout_input(AssistantTimeout::Text("7.5".to_owned()), None);
    let (_, result) = recorded(&input).await;
    assert_eq!(result.call.timeout, FineTuningTimeout::Seconds(7.5));
}

async fn a_structured_timeout_keeps_its_read_phase() {
    let input = timeout_input(
        AssistantTimeout::Structured(TimeoutPhases {
            connect: Some(2.0),
            read: Some(9.0),
            write: None,
            pool: None,
        }),
        None,
    );
    let (_, result) = recorded(&input).await;
    assert_eq!(
        result.call.timeout,
        FineTuningTimeout::Structured(StructuredTimeout {
            connect: Some(2.0),
            read: Some(9.0),
            write: None,
            pool: None,
        })
    );
    assert_eq!(
        openai_timeout(&result),
        Some(Duration::from_secs(9)),
        "a structured timeout reaches the OpenAI SDK as its read phase"
    );
}

#[tokio::test]
async fn timeout_resolution_matches_the_facade_chain() {
    the_default_timeout_is_ten_minutes().await;
    a_number_wins_over_everything().await;
    a_zero_timeout_falls_to_the_request_timeout().await;
    an_empty_text_timeout_falls_to_the_request_timeout().await;
    a_text_timeout_is_parsed_like_float().await;
    a_structured_timeout_keeps_its_read_phase().await;
}

#[tokio::test]
async fn openai_retry_budget_clamps_negative_values() {
    let mut negative_input = list_openai(AssistantWrapper::Sync);
    negative_input.params.max_retries = Some(-2);
    let (_, negative_result) = recorded(&negative_input).await;
    assert_eq!(openai_retries(&negative_result), Some(0));

    let mut positive_input = list_openai(AssistantWrapper::Sync);
    positive_input.params.max_retries = Some(5);
    let (_, positive_result) = recorded(&positive_input).await;
    assert_eq!(
        openai_retries(&positive_result),
        Some(5),
        "a positive value reaches the SDK unchanged"
    );
}

fn env_token_settings() -> Settings {
    Settings {
        azure_ad_token: Some(SecretString::from("env-token")),
        ..Settings::default()
    }
}

async fn azure_call(input: &AssistantReadDeleteInput) -> AssistantReadDeleteResult {
    let (_, dispatch) = harness(200, DELETED);
    facade(dispatch, env_token_settings())
        .run(input)
        .await
        .unwrap()
}

async fn kwargs_token_stays_in_litellm_params() {
    let mut input = delete_azure(AssistantWrapper::Sync);
    input.params.azure_ad_token = Some(SecretString::from("kwarg-token"));
    input.params.max_retries = Some(4);
    let result = azure_call(&input).await;
    let values = azure_values(&result);
    assert!(values.azure_ad_token.is_none());
    assert_eq!(
        exposed(values.params.azure_ad_token.as_ref()),
        Some("kwarg-token")
    );
    assert_eq!(values.params.max_retries, Some(4));
    assert_eq!(
        values.params.timeout, None,
        "an absent kwargs timeout stays absent"
    );
    assert!(values.extra_body.is_empty());
}

async fn a_number_timeout_reaches_litellm_params() {
    let mut input = delete_azure(AssistantWrapper::Sync);
    input.params.timeout = Some(AssistantTimeout::Number(9.5));
    let result = azure_call(&input).await;
    assert_eq!(azure_values(&result).params.timeout, Some(9.5));
}

async fn a_text_timeout_reaches_litellm_params() {
    let mut input = delete_azure(AssistantWrapper::Sync);
    input.params.timeout = Some(AssistantTimeout::Text(" 7.5 ".to_owned()));
    let result = azure_call(&input).await;
    assert_eq!(azure_values(&result).params.timeout, Some(7.5));
}

async fn a_structured_timeout_skips_litellm_params() {
    let mut input = delete_azure(AssistantWrapper::Sync);
    input.params.timeout = Some(AssistantTimeout::Structured(TimeoutPhases {
        connect: Some(1.0),
        read: None,
        write: None,
        pool: None,
    }));
    let result = azure_call(&input).await;
    assert_eq!(
        azure_values(&result).params.timeout,
        None,
        "an httpx.Timeout leaves litellm_params without one"
    );
}

async fn a_null_extra_body_uses_the_env_token() {
    let mut input = delete_azure(AssistantWrapper::Sync);
    input.extra_body = Some(None);
    let result = azure_call(&input).await;
    let values = azure_values(&result);
    assert_eq!(exposed(values.azure_ad_token.as_ref()), Some("env-token"));
}

async fn the_body_token_wins_and_is_removed() {
    let body = indexmap::IndexMap::from([
        (
            "azure_ad_token".to_owned(),
            Json::Text("body-token".to_owned()),
        ),
        ("other".to_owned(), Json::Bool(true)),
    ]);
    let mut input = delete_azure(AssistantWrapper::Sync);
    input.extra_body = Some(Some(body));
    let result = azure_call(&input).await;
    let values = azure_values(&result);
    assert_eq!(exposed(values.azure_ad_token.as_ref()), Some("body-token"));
    assert!(!values.extra_body.contains_key("azure_ad_token"));
    assert_eq!(values.extra_body.get("other"), Some(&Json::Bool(true)));
}

async fn an_empty_extra_body_leaves_no_token() {
    let mut input = delete_azure(AssistantWrapper::Sync);
    input.extra_body = Some(Some(indexmap::IndexMap::new()));
    let result = azure_call(&input).await;
    assert!(azure_values(&result).azure_ad_token.is_none());
}

#[tokio::test]
async fn azure_extra_body_reads_and_removes_the_token() {
    kwargs_token_stays_in_litellm_params().await;
    a_number_timeout_reaches_litellm_params().await;
    a_text_timeout_reaches_litellm_params().await;
    a_structured_timeout_skips_litellm_params().await;
    a_null_extra_body_uses_the_env_token().await;
    the_body_token_wins_and_is_removed().await;
    an_empty_extra_body_leaves_no_token().await;
}

fn no_request() -> Sdk {
    harness(200, PAGE).1
}

async fn a_list_flag_that_is_not_a_bool_is_an_exception() {
    let mut input = list_openai(AssistantWrapper::Sync);
    input.aget_assistants = Some(Json::Number(serde_json::Number::from(1)));
    let error = facade(no_request(), Settings::default())
        .run(&input)
        .await
        .unwrap_err();
    let fields = error.fields().unwrap();
    assert_eq!(fields.class, "Exception");
    assert_eq!(fields.status, None);
    assert_eq!(fields.error_type, None);
    assert_eq!(
        error.to_string(),
        "Invalid value passed in for aget_assistants. Only bool or None allowed"
    );
}

async fn a_delete_flag_that_is_not_a_bool_is_a_value_error() {
    let mut input = delete_azure(AssistantWrapper::Sync);
    input.async_delete_assistants = Some(Json::Text("true".to_owned()));
    let error = facade(no_request(), Settings::default())
        .run(&input)
        .await
        .unwrap_err();
    let fields = error.fields().unwrap();
    assert_eq!(fields.class, "ValueError");
    assert_eq!(
        error.to_string(),
        "Invalid value passed in for async_delete_assistants. Only bool or None allowed"
    );
}

async fn the_async_entry_ignores_the_callers_invalid_flag() {
    let mut input = delete_azure(AssistantWrapper::Sync);
    input.async_delete_assistants = Some(Json::Text("true".to_owned()));
    input.entry = AssistantWrapper::Async;
    let (recorder, dispatch) = harness(200, DELETED);
    facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_eq!(recorder.requests().len(), 1);
}

#[tokio::test]
async fn invalid_flags_raise_the_litellm_classes_without_a_request() {
    a_list_flag_that_is_not_a_bool_is_an_exception().await;
    a_delete_flag_that_is_not_a_bool_is_a_value_error().await;
    the_async_entry_ignores_the_callers_invalid_flag().await;
}

async fn an_unsupported_list_provider_is_a_bad_request() {
    let mut input = list_openai(AssistantWrapper::Sync);
    input.provider = "anthropic".to_owned();
    let error = facade(no_request(), Settings::default())
        .run(&input)
        .await
        .unwrap_err();
    let fields = error.fields().unwrap();
    assert_eq!(fields.class, "BadRequestError");
    assert_eq!(fields.status, Some(400));
    assert_eq!(fields.error_type, None);
    assert_eq!(
        error.to_string(),
        "LiteLLM doesn't support anthropic for 'get_assistants'. Only 'openai' is supported."
    );
}

async fn an_unsupported_delete_provider_names_delete_assistant() {
    let mut input = delete_azure(AssistantWrapper::Sync);
    input.provider = "anthropic".to_owned();
    let error = facade(no_request(), Settings::default())
        .run(&input)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "LiteLLM doesn't support anthropic for 'delete_assistant'. Only 'openai' is supported."
    );
}

async fn a_null_list_reply_repeats_the_bad_request() {
    let scripted = Scripted { reply: Ok(None) };
    let input = list_openai(AssistantWrapper::Sync);
    let error = facade(scripted.into_dispatch(), Settings::default())
        .run(&input)
        .await
        .unwrap_err();
    assert_eq!(error.fields().unwrap().status, Some(400));
    assert_eq!(
        error.to_string(),
        "LiteLLM doesn't support openai for 'get_assistants'. Only 'openai' is supported."
    );
}

async fn a_null_delete_reply_is_an_internal_server_error() {
    let scripted = Scripted { reply: Ok(None) };
    let mut input = list_openai(AssistantWrapper::Sync);
    input.operation = AssistantOperation::Delete;
    input.assistant_id = Some("asst_a".to_owned());
    let error = facade(scripted.into_dispatch(), Settings::default())
        .run(&input)
        .await
        .unwrap_err();
    let fields = error.fields().unwrap();
    assert_eq!(fields.class, "InternalServerError");
    assert_eq!(fields.status, Some(500));
    assert_eq!(fields.error_type, Some("internal_server_error"));
    assert_eq!(
        error.to_string(),
        "No response returned from 'delete_assistant'"
    );
}

async fn a_reply_of_the_wrong_kind_is_a_dispatch_bug() {
    let scripted = Scripted {
        reply: Ok(Some(AssistantReply::Page(page_fixture()))),
    };
    let mut input = list_openai(AssistantWrapper::Sync);
    input.operation = AssistantOperation::Delete;
    input.assistant_id = Some("asst_a".to_owned());
    let error = facade(scripted.into_dispatch(), Settings::default())
        .run(&input)
        .await
        .unwrap_err();
    assert_eq!(
        error.fields().unwrap().class,
        "InternalServerError",
        "a reply of the other operation's kind is a dispatch bug, not a page to hand back"
    );
}

async fn a_delete_without_an_id_fails_before_dispatch() {
    let scripted = Scripted { reply: Ok(None) };
    let mut input = delete_azure(AssistantWrapper::Sync);
    input.assistant_id = None;
    let error = facade(scripted.into_dispatch(), Settings::default())
        .run(&input)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AssistantReadDeleteError::MissingAssistantId
    ));
}

#[tokio::test]
async fn unsupported_providers_and_null_replies_answer_like_litellm() {
    an_unsupported_list_provider_is_a_bad_request().await;
    an_unsupported_delete_provider_names_delete_assistant().await;
    a_null_list_reply_repeats_the_bad_request().await;
    a_null_delete_reply_is_an_internal_server_error().await;
    a_reply_of_the_wrong_kind_is_a_dispatch_bug().await;
    a_delete_without_an_id_fails_before_dispatch().await;
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("scripted failure")]
struct ScriptFailure;

struct Scripted {
    reply: Result<Option<AssistantReply>, ScriptFailure>,
}

impl Scripted {
    fn into_dispatch(self) -> Recording {
        Recording { reply: self.reply }
    }
}

struct Recording {
    reply: Result<Option<AssistantReply>, ScriptFailure>,
}

impl AssistantReadDeleteDispatch for Recording {
    type Error = ScriptFailure;

    fn call(
        &self,
        _call: &AssistantCall,
    ) -> impl Future<Output = Result<Option<AssistantReply>, ScriptFailure>> + Send {
        let reply = self.reply.clone();
        async move { reply }
    }
}

#[tokio::test]
async fn provider_failures_keep_their_status_and_body() {
    let (recorder, dispatch) = harness(404, NOT_FOUND_BODY);
    let input = list_openai(AssistantWrapper::Sync);
    let error = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap_err();
    let AssistantReadDeleteError::Dispatch(AssistantSdkError::Assistants(
        AssistantsError::Status { status, body },
    )) = &error
    else {
        panic!("expected a status failure, got {error:?}");
    };
    assert_eq!(*status, 404);
    assert_eq!(body.as_slice(), NOT_FOUND_BODY.as_bytes());
    assert_eq!(recorder.requests().len(), 1);
}

#[tokio::test]
async fn limits_refuse_an_oversized_extra_body() {
    let limits = ApiLimits {
        max_items: 0,
        ..ApiLimits::default()
    };
    let limited = AssistantReadDelete {
        dispatch: no_request(),
        limits,
        settings: Settings::default(),
    };
    let mut input = delete_azure(AssistantWrapper::Sync);
    input.extra_body = Some(Some(indexmap::IndexMap::from([(
        "key".to_owned(),
        Json::Text("value".to_owned()),
    )])));
    let error = limited.run(&input).await.unwrap_err();
    assert!(matches!(error, AssistantReadDeleteError::Limits(_)));
    assert!(error.fields().is_none());
}

#[tokio::test]
async fn the_async_entry_forces_the_flag_even_when_the_caller_falses_it() {
    let (recorder, dispatch) = harness(200, PAGE);
    let mut input = list_openai(AssistantWrapper::Async);
    input.aget_assistants = Some(Json::Bool(false));
    facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_eq!(recorder.requests().len(), 1);
}

// an OIDC assertion reaches sources.oidc in credentials/selection.rs, and these dispatches configure none
#[test]
fn no_sources_name_the_missing_credential_source() {
    let error = NoSources
        .oidc(&OidcExchangeInput {
            azure_ad_token: SecretString::from("assertion"),
            azure_client_id: Some("client".to_owned()),
            azure_tenant_id: Some("tenant".to_owned()),
            scope: None,
        })
        .unwrap_err();
    assert_eq!(error.class, "ValueError");
    assert_eq!(error.status, None);
    assert_eq!(error.message, "no oidc credential source is configured");
}
