#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::sections::azure_endpoints::Settings as AzureSettings;
use betterlite_config::sections::resource_sdk_settings::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_prompts::arguments::json::Json;
use http::HeaderMap;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::call::AssistantProviderCall;
use super::error::AssistantReadDeleteError;
use super::request::{
    AssistantFlag, AssistantOperation, AssistantParams, AssistantReadDeleteInput, AssistantTimeout,
    AssistantWrapper, SuppliedClient, SuppliedClientKind, TimeoutPhases,
};
use super::sdk_dispatch::{NoSources, SdkDispatch};
use super::{AssistantReadDelete, AssistantStep};
use crate::api::resource_sdk_settings::timeout::{FineTuningTimeout, StructuredTimeout};

const PAGE: &str = r#"{"object":"list","data":[{"id":"asst_b","object":"assistant","created_at":1,"model":"gpt-4o","tools":[]}],"has_more":true,"first_id":"asst_b","last_id":"asst_b"}"#;
const DELETED: &str = r#"{"id":"asst_a","object":"assistant.deleted","deleted":true}"#;
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
            .unwrap_or((500, String::new()));
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

fn facade(dispatch: Sdk, settings: Settings) -> AssistantReadDelete<Sdk> {
    AssistantReadDelete {
        dispatch,
        limits: super::ApiLimits::default(),
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

fn page_step(step: AssistantStep) -> Value {
    match step {
        AssistantStep::Page(page) => serde_json::to_value(page).unwrap(),
        other => panic!("expected a page, got {other:?}"),
    }
}

#[tokio::test]
async fn list_drops_pagination_and_sends_the_sdk_defaults() {
    let (recorder, dispatch) = harness(200, PAGE);
    let mut input = list_openai(AssistantWrapper::Async);
    input.after = Some("asst_a".to_owned());
    input.before = Some("asst_z".to_owned());
    input.limit = Some(3);
    input.order = Some("asc".to_owned());
    let result = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    let requests = recorder.requests();
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
    assert_eq!(
        page_step(result.step),
        serde_json::from_str::<Value>(PAGE).unwrap()
    );
}

#[tokio::test]
async fn sync_list_without_a_flag_makes_the_same_request() {
    let (recorder, dispatch) = harness(200, PAGE);
    let input = list_openai(AssistantWrapper::Sync);
    let result = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_eq!(recorder.requests().len(), 1);
    assert!(
        recorder.requests()[0]
            .url
            .ends_with("/v1/assistants?limit=20&order=desc")
    );
    assert_eq!(
        page_step(result.step),
        serde_json::from_str::<Value>(PAGE).unwrap()
    );
}

#[tokio::test]
async fn sync_list_with_the_async_flag_hands_back_a_coroutine() {
    let (recorder, dispatch) = harness(200, PAGE);
    let mut input = list_openai(AssistantWrapper::Sync);
    input.aget_assistants = Some(AssistantFlag::Bool(true));
    let result = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_eq!(result.step, AssistantStep::Coroutine);
    assert!(recorder.requests().is_empty());
}

#[tokio::test]
async fn azure_list_keeps_api_version_and_drops_pagination() {
    let (recorder, dispatch) = harness(200, PAGE);
    let mut input = delete_azure(AssistantWrapper::Async);
    input.operation = AssistantOperation::List;
    input.assistant_id = None;
    input.after = Some("asst_a".to_owned());
    input.order = Some("asc".to_owned());
    let result = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    let requests = recorder.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url,
        "https://azure.test/v1/openai/assistants?api-version=2024-02-01"
    );
    assert_eq!(requests[0].api_key.as_deref(), Some("sk-request"));
    assert_eq!(requests[0].beta.as_deref(), Some("assistants=v2"));
    assert_eq!(
        page_step(result.step),
        serde_json::from_str::<Value>(PAGE).unwrap()
    );
}

fn assert_delete(seen: &Seen, step: AssistantStep) {
    assert_eq!(seen.method, "DELETE");
    assert_eq!(
        seen.url,
        "https://azure.test/v1/openai/assistants/asst_a?api-version=2024-02-01"
    );
    match step {
        AssistantStep::Deleted(deleted) => {
            assert_eq!(
                serde_json::to_value(deleted).unwrap(),
                json!({
                    "id": "asst_a",
                    "object": "assistant.deleted",
                    "deleted": true,
                })
            );
        }
        other => panic!("expected a deleted object, got {other:?}"),
    }
}

#[tokio::test]
async fn delete_sends_the_identifier_in_both_entries() {
    let input = delete_azure(AssistantWrapper::Sync);
    let (recorder, dispatch) = harness(200, DELETED);
    let sync = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_delete(&recorder.requests()[0], sync.step);

    let input = delete_azure(AssistantWrapper::Async);
    let (recorder, dispatch) = harness(200, DELETED);
    let asynchronous = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_delete(&recorder.requests()[0], asynchronous.step);
}

async fn assert_azure_delete_discards(kind: SuppliedClientKind) {
    let (recorder, dispatch) = harness(200, DELETED);
    let mut input = delete_azure(AssistantWrapper::Async);
    input.client = Some(supplied(kind, AZURE_BASE));
    facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    let requests = recorder.requests();
    assert_eq!(requests[0].api_key.as_deref(), Some("sk-request"));
    assert_eq!(requests[0].authorization, None);
}

#[tokio::test]
async fn azure_delete_discards_openai_class_clients_and_keeps_async_ones() {
    assert_azure_delete_discards(SuppliedClientKind::OpenAi).await;
    assert_azure_delete_discards(SuppliedClientKind::AzureOpenAi).await;

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

#[tokio::test]
async fn azure_list_and_openai_delete_keep_openai_class_clients() {
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
    assert_eq!(requests[0].api_key, None);

    let (recorder, dispatch) = harness(200, PAGE);
    let mut input = delete_azure(AssistantWrapper::Sync);
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
    assert_eq!(requests[0].authorization, None);

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

    let (recorder, dispatch) = harness(200, DELETED);
    let mut input = list_openai(AssistantWrapper::Sync);
    input.operation = AssistantOperation::Delete;
    input.assistant_id = Some("asst_a".to_owned());
    input.client = Some(supplied(SuppliedClientKind::OpenAi, OPENAI_BASE));
    facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_eq!(
        recorder.requests()[0].url,
        format!("{OPENAI_BASE}/assistants/asst_a")
    );
    assert_eq!(
        recorder.requests()[0].authorization.as_deref(),
        Some("Bearer sk-supplied")
    );
}

#[tokio::test]
async fn openai_credentials_follow_the_litellm_chain() {
    let mut settings = Settings::default();
    settings.api_base = Some("https://settings.test/v1".to_owned());
    settings.api_key = Some(SecretString::from("sk-settings"));
    settings.openai_base_url = Some("https://env.test/v1".to_owned());
    settings.openai_api_key = Some(SecretString::from("sk-env"));
    settings.organization = Some("org-settings".to_owned());
    settings.openai_organization = Some("org-env".to_owned());
    let (recorder, dispatch) = harness(200, PAGE);
    let mut input = list_openai(AssistantWrapper::Sync);
    input.params.api_base = Some(String::new());
    input.params.api_key = None;
    facade(dispatch, settings.clone())
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

    let empty = Settings {
        api_base: Some(String::new()),
        api_key: Some(SecretString::from("")),
        organization: Some(String::new()),
        ..settings
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

    let (recorder, dispatch) = harness(200, PAGE);
    let mut input = list_openai(AssistantWrapper::Sync);
    input.params.api_base = None;
    input.params.api_key = None;
    facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_eq!(
        recorder.requests()[0].url,
        "https://api.openai.com/v1/assistants?limit=20&order=desc"
    );
    assert_eq!(recorder.requests()[0].authorization, None);
}

#[tokio::test]
async fn timeout_resolution_matches_the_facade_chain() {
    let (_, dispatch) = harness(200, PAGE);
    let input = list_openai(AssistantWrapper::Sync);
    let result = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_eq!(result.call.timeout, FineTuningTimeout::Seconds(600.0));
    assert_eq!(
        openai_timeout(&result),
        Some(Duration::from_secs(600)),
        "the OpenAI SDK gets the resolved default"
    );

    let mut input = list_openai(AssistantWrapper::Sync);
    input.params.timeout = Some(AssistantTimeout::Number(9.5));
    let (_, dispatch) = harness(200, PAGE);
    let result = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_eq!(result.call.timeout, FineTuningTimeout::Seconds(9.5));

    let mut input = list_openai(AssistantWrapper::Sync);
    input.params.timeout = Some(AssistantTimeout::Number(0.0));
    input.params.request_timeout = Some(8.0);
    let (_, dispatch) = harness(200, PAGE);
    let result = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_eq!(result.call.timeout, FineTuningTimeout::Seconds(8.0));

    let mut input = list_openai(AssistantWrapper::Sync);
    input.params.timeout = Some(AssistantTimeout::Text(String::new()));
    input.params.request_timeout = Some(8.0);
    let (_, dispatch) = harness(200, PAGE);
    let result = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_eq!(result.call.timeout, FineTuningTimeout::Seconds(8.0));

    let mut input = list_openai(AssistantWrapper::Sync);
    input.params.timeout = Some(AssistantTimeout::Text("7.5".to_owned()));
    let (_, dispatch) = harness(200, PAGE);
    let result = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_eq!(result.call.timeout, FineTuningTimeout::Seconds(7.5));

    let mut input = list_openai(AssistantWrapper::Sync);
    input.params.timeout = Some(AssistantTimeout::Structured(TimeoutPhases {
        connect: Some(2.0),
        read: Some(9.0),
        write: None,
        pool: None,
    }));
    let (_, dispatch) = harness(200, PAGE);
    let result = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
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
async fn openai_retry_budget_clamps_negative_values() {
    let (_, dispatch) = harness(200, PAGE);
    let mut input = list_openai(AssistantWrapper::Sync);
    input.params.max_retries = Some(-2);
    let result = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_eq!(openai_retries(&result), Some(0));

    let (_, dispatch) = harness(200, PAGE);
    let mut input = list_openai(AssistantWrapper::Sync);
    input.params.max_retries = Some(5);
    let result = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_eq!(
        openai_retries(&result),
        Some(5),
        "a positive value reaches the SDK unchanged"
    );
}

fn openai_timeout(result: &super::AssistantReadDeleteResult) -> Option<Duration> {
    let AssistantProviderCall::OpenAi(options) = &result.call.provider else {
        panic!("expected the openai branch");
    };
    options.timeout
}

fn openai_retries(result: &super::AssistantReadDeleteResult) -> Option<u32> {
    let AssistantProviderCall::OpenAi(options) = &result.call.provider else {
        panic!("expected the openai branch");
    };
    options.max_retries
}

#[tokio::test]
async fn azure_extra_body_reads_and_removes_the_token() {
    let mut settings = Settings::default();
    settings.azure_ad_token = Some(SecretString::from("env-token"));
    let (_, dispatch) = harness(200, PAGE);
    let mut input = delete_azure(AssistantWrapper::Sync);
    input.params.azure_ad_token = Some(SecretString::from("kwarg-token"));
    input.params.max_retries = Some(4);
    let result = facade(dispatch, settings.clone())
        .run(&input)
        .await
        .unwrap();
    let AssistantProviderCall::Azure(values) = &result.call.provider else {
        panic!("expected the azure branch");
    };
    assert_eq!(values.azure_ad_token, None);
    assert_eq!(
        values
            .params
            .azure_ad_token
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("kwarg-token")
    );
    assert_eq!(values.params.max_retries, Some(4));
    assert!(values.extra_body.is_empty());

    let (_, dispatch) = harness(200, PAGE);
    let mut input = delete_azure(AssistantWrapper::Sync);
    input.extra_body = Some(None);
    let result = facade(dispatch, settings.clone())
        .run(&input)
        .await
        .unwrap();
    let AssistantProviderCall::Azure(values) = &result.call.provider else {
        panic!("expected the azure branch");
    };
    assert_eq!(
        values
            .azure_ad_token
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("env-token")
    );

    let body = indexmap::IndexMap::from([
        (
            "azure_ad_token".to_owned(),
            Json::Text("body-token".to_owned()),
        ),
        ("other".to_owned(), Json::Bool(true)),
    ]);
    let (_, dispatch) = harness(200, PAGE);
    let mut input = delete_azure(AssistantWrapper::Sync);
    input.extra_body = Some(Some(body));
    let result = facade(dispatch, settings.clone())
        .run(&input)
        .await
        .unwrap();
    let AssistantProviderCall::Azure(values) = &result.call.provider else {
        panic!("expected the azure branch");
    };
    assert_eq!(
        values
            .azure_ad_token
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        Some("body-token")
    );
    assert!(!values.extra_body.contains_key("azure_ad_token"));
    assert_eq!(values.extra_body.get("other"), Some(&Json::Bool(true)));

    let (_, dispatch) = harness(200, PAGE);
    let mut input = delete_azure(AssistantWrapper::Sync);
    input.extra_body = Some(Some(indexmap::IndexMap::new()));
    let result = facade(dispatch, settings).run(&input).await.unwrap();
    let AssistantProviderCall::Azure(values) = &result.call.provider else {
        panic!("expected the azure branch");
    };
    assert_eq!(values.azure_ad_token, None);
}

#[tokio::test]
async fn invalid_flags_raise_the_litellm_classes_without_a_request() {
    let mut input = list_openai(AssistantWrapper::Sync);
    input.aget_assistants = Some(AssistantFlag::Other(Json::Number(
        serde_json::Number::from(1),
    )));
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

    let mut input = delete_azure(AssistantWrapper::Sync);
    input.async_delete_assistants = Some(AssistantFlag::Other(Json::Text("true".to_owned())));
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

    let mut input = delete_azure(AssistantWrapper::Sync);
    input.async_delete_assistants = Some(AssistantFlag::Other(Json::Text("true".to_owned())));
    input.entry = AssistantWrapper::Async;
    let (recorder, dispatch) = harness(200, DELETED);
    facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_eq!(recorder.requests().len(), 1);
}

fn no_request() -> Sdk {
    let (_, dispatch) = harness(200, PAGE);
    dispatch
}

#[tokio::test]
async fn unsupported_providers_and_null_replies_answer_like_litellm() {
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

    let mut input = list_openai(AssistantWrapper::Sync);
    let scripted = Scripted { reply: Ok(None) };
    let error = facade(scripted.into_dispatch(), Settings::default())
        .run(&input)
        .await
        .unwrap_err();
    assert_eq!(error.fields().unwrap().status, Some(400));
    assert_eq!(
        error.to_string(),
        "LiteLLM doesn't support openai for 'get_assistants'. Only 'openai' is supported."
    );

    input.operation = AssistantOperation::Delete;
    input.assistant_id = Some("asst_a".to_owned());
    let scripted = Scripted { reply: Ok(None) };
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

    let page = serde_json::from_str::<betterlite_openai::assistants::records::AssistantPage>(PAGE)
        .unwrap();
    let scripted = Scripted {
        reply: Ok(Some(super::AssistantReply::Page(page))),
    };
    let error = facade(scripted.into_dispatch(), Settings::default())
        .run(&input)
        .await
        .unwrap_err();
    assert_eq!(
        error.fields().unwrap().class,
        "InternalServerError",
        "a reply of the other operation's kind is a dispatch bug, not a page to hand back"
    );

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

#[derive(Debug, thiserror::Error)]
#[error("scripted failure")]
struct ScriptFailure;

struct Scripted {
    reply: Result<Option<super::AssistantReply>, ScriptFailure>,
}

impl Scripted {
    fn into_dispatch(self) -> Recording {
        Recording {
            reply: self.reply,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

struct Recording {
    reply: Result<Option<super::AssistantReply>, ScriptFailure>,
    calls: Arc<Mutex<Vec<super::call::AssistantCall>>>,
}

impl super::AssistantReadDeleteDispatch for Recording {
    type Error = ScriptFailure;

    fn call(
        &self,
        call: &super::call::AssistantCall,
    ) -> impl Future<Output = Result<Option<super::AssistantReply>, ScriptFailure>> + Send {
        self.calls.lock().unwrap().push(call.clone());
        let reply = self.reply.clone();
        async move { reply }
    }
}

#[tokio::test]
async fn provider_failures_keep_their_status_and_body() {
    let body = r#"{"error":{"message":"missing","type":"invalid_request_error","param":"assistant_id","code":"missing"}}"#;
    let (recorder, dispatch) = harness(404, body);
    let input = list_openai(AssistantWrapper::Sync);
    let error = facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap_err();
    match error {
        AssistantReadDeleteError::Dispatch(super::sdk_dispatch::AssistantSdkError::Assistants(
            betterlite_openai::assistants::error::AssistantsError::Status { status, body },
        )) => {
            assert_eq!(status, 404);
            assert_eq!(
                String::from_utf8(body).unwrap(),
                r#"{"error":{"message":"missing","type":"invalid_request_error","param":"assistant_id","code":"missing"}}"#
            );
        }
        other => panic!("expected a status failure, got {other:?}"),
    }
    assert_eq!(recorder.requests().len(), 1);
}

#[tokio::test]
async fn limits_refuse_an_oversized_extra_body() {
    let mut limits = super::ApiLimits::default();
    limits.max_items = 0;
    let facade = AssistantReadDelete {
        dispatch: no_request(),
        limits,
        settings: Settings::default(),
    };
    let mut input = delete_azure(AssistantWrapper::Sync);
    input.extra_body = Some(Some(indexmap::IndexMap::from([(
        "key".to_owned(),
        Json::Text("value".to_owned()),
    )])));
    let error = facade.run(&input).await.unwrap_err();
    assert!(matches!(error, AssistantReadDeleteError::Limits(_)));
    assert!(error.fields().is_none());
}

#[tokio::test]
async fn the_async_entry_forces_the_flag_even_when_the_caller_falses_it() {
    let (recorder, dispatch) = harness(200, PAGE);
    let mut input = list_openai(AssistantWrapper::Async);
    input.aget_assistants = Some(AssistantFlag::Bool(false));
    facade(dispatch, Settings::default())
        .run(&input)
        .await
        .unwrap();
    assert_eq!(recorder.requests().len(), 1);
}
