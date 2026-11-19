#![cfg(test)]

use std::future::Future;
use std::io;
use std::iter;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::fine_tuning::job::{
    FineTuneCreate, FineTuneJob, Hyperparameters, JobStatus, decode_fine_tune_job,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
use thiserror::Error;
use tokio::task::{Id, LocalSet, spawn_local, try_id};
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::connection::{CreateParams, CreateTimeout};
use super::service::{FineTuningCreate, prepare_azure_extra_body};
use super::{
    AzureCreateCall, CreateFineTuningJobCall, FineTuningCreateDispatch, FineTuningCreateError,
    FineTuningCreateTarget, OpenAiCreateCall, UNSUPPORTED_MODEL, UNSUPPORTED_REQUEST_METHOD,
    UNSUPPORTED_REQUEST_URL, UNSUPPORTED_RESPONSE_BODY, UnsupportedProviderError, VertexCreateCall,
};
use crate::api::resource_sdk_settings::ResourceSdkSettings;
use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, TimeoutValue,
};

const JOB: &str = r#"{"id":"ft_a","object":"fine_tuning.job","created_at":1704067200,"model":"gpt-4o-mini","organization_id":"org-1","result_files":[],"status":"queued","training_file":"file_train","hyperparameters":{"batch_size":null,"learning_rate_multiplier":0.5,"n_epochs":3}}"#;

#[derive(Debug, Clone, Error)]
#[error("{message}")]
struct ScriptFailure {
    message: String,
}

fn job() -> FineTuneJob {
    decode_fine_tune_job(JOB.as_bytes(), false).expect("the fixture job decodes")
}

struct Scripted {
    calls: Arc<Mutex<Vec<FineTuningCreateTarget>>>,
}

impl FineTuningCreateDispatch for Scripted {
    type Error = ScriptFailure;

    fn create(
        &self,
        target: FineTuningCreateTarget,
    ) -> impl Future<Output = Result<FineTuneJob, ScriptFailure>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(target);
        async move { Ok(job()) }
    }
}

type Ran = (
    Result<FineTuneJob, FineTuningCreateError<ScriptFailure>>,
    Vec<FineTuningCreateTarget>,
);

async fn ran(settings: ResourceSdkSettings, call: &CreateFineTuningJobCall) -> Ran {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let dispatch = Scripted {
        calls: Arc::clone(&calls),
    };
    let outcome = FineTuningCreate { dispatch, settings }.create(call).await;
    let recorded = calls.lock().unwrap_or_else(PoisonError::into_inner).clone();
    (outcome, recorded)
}

fn settings() -> ResourceSdkSettings {
    ResourceSdkSettings {
        api_base: None,
        api_version: None,
        api_key: None,
        openai_key: None,
        azure_key: None,
        azure_ad_token: None,
        organization: None,
        vertex_project: None,
        vertex_location: None,
        openai_base_url: None,
        openai_api_base: None,
        openai_api_key: None,
        openai_organization: None,
        azure_api_base: None,
        azure_api_version: None,
        azure_openai_api_key: None,
        azure_api_key: None,
        vertexai_project: None,
        vertexai_location: None,
        vertexai_credentials: None,
    }
}

fn base_call(provider: &str) -> CreateFineTuningJobCall {
    CreateFineTuningJobCall {
        model: "gpt-4o-mini".to_owned(),
        training_file: "file_train".to_owned(),
        hyperparameters: None,
        suffix: None,
        validation_file: None,
        integrations: None,
        seed: None,
        custom_llm_provider: provider.to_owned(),
        extra_headers: None,
        extra_body: None,
        params: CreateParams::default(),
        is_async: false,
    }
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn object(entries: &[(&str, Json)]) -> Extra {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn body(data: &FineTuneCreate) -> Extra {
    serde_json::from_str(&serde_json::to_string(data).expect("the job data encodes"))
        .expect("the body is an object")
}

fn only_target(mut calls: Vec<FineTuningCreateTarget>) -> FineTuningCreateTarget {
    assert_eq!(calls.len(), 1);
    calls.remove(0)
}

fn openai_target(provider: &str, calls: Vec<FineTuningCreateTarget>) -> OpenAiCreateCall {
    match only_target(calls) {
        FineTuningCreateTarget::OpenAi(call) => call,
        other @ (FineTuningCreateTarget::Azure(_) | FineTuningCreateTarget::Vertex(_)) => {
            panic!("{provider} built {other:?}")
        }
    }
}

fn azure_target(calls: Vec<FineTuningCreateTarget>) -> AzureCreateCall {
    match only_target(calls) {
        FineTuningCreateTarget::Azure(call) => call,
        other @ (FineTuningCreateTarget::OpenAi(_) | FineTuningCreateTarget::Vertex(_)) => {
            panic!("azure built {other:?}")
        }
    }
}

fn vertex_target(calls: Vec<FineTuningCreateTarget>) -> VertexCreateCall {
    match only_target(calls) {
        FineTuningCreateTarget::Vertex(call) => call,
        other @ (FineTuningCreateTarget::OpenAi(_) | FineTuningCreateTarget::Azure(_)) => {
            panic!("vertex built {other:?}")
        }
    }
}

fn secret(value: &SecretString) -> &str {
    value.expose_secret()
}

async fn openai_defaults() {
    let (outcome, calls) = ran(settings(), &base_call("openai")).await;
    let returned = outcome.expect("the provider answered");
    assert_eq!(returned.id, "ft_a");
    assert_eq!(returned.status, JobStatus::Queued);
    assert_eq!(returned.model, "gpt-4o-mini");
    let call = openai_target("openai", calls);
    assert!(!call.is_async);
    assert_eq!(call.api_base, "https://api.openai.com/v1");
    let fields = body(&call.data);
    assert_eq!(fields.get("model"), Some(&text("gpt-4o-mini")));
    assert_eq!(fields.get("training_file"), Some(&text("file_train")));
    assert_eq!(
        fields.get("hyperparameters"),
        Some(&Json::Object(Extra::new()))
    );
    assert_eq!(fields.len(), 3);
    let mut asynchronous = base_call("openai");
    asynchronous.is_async = true;
    let (asynchronous_outcome, asynchronous_calls) = ran(settings(), &asynchronous).await;
    asynchronous_outcome.expect("the provider answered");
    assert!(openai_target("openai", asynchronous_calls).is_async);
}

async fn openai_supplied() {
    let mut call = base_call("openai");
    call.hyperparameters = Some(object(&[
        ("batch_size", text("auto")),
        ("n_epochs", number(3)),
        ("learning_rate_multiplier", number(1)),
        ("extra_flag", text("x")),
        // the Azure-only extraction never touches a non-Azure caller's map
        ("prompt_loss_weight", number(1)),
    ]));
    call.suffix = Some(String::new());
    call.validation_file = Some(String::new());
    call.integrations = Some(Vec::new());
    call.seed = Some(0);
    let (outcome, calls) = ran(settings(), &call).await;
    outcome.expect("the provider answered");
    let fields = body(&openai_target("openai", calls).data);
    let hyperparameters = match fields.get("hyperparameters") {
        Some(Json::Object(entries)) => entries.clone(),
        other => panic!("unexpected hyperparameters {other:?}"),
    };
    assert_eq!(hyperparameters.get("batch_size"), Some(&text("auto")));
    assert_eq!(hyperparameters.get("n_epochs"), Some(&number(3)));
    assert_eq!(hyperparameters.get("extra_flag"), Some(&text("x")));
    assert_eq!(hyperparameters.get("prompt_loss_weight"), Some(&number(1)));
    assert_eq!(fields.get("suffix"), Some(&text("")));
    assert_eq!(fields.get("validation_file"), Some(&text("")));
    assert_eq!(fields.get("integrations"), Some(&Json::List(Vec::new())));
    assert_eq!(fields.get("seed"), Some(&number(0)));
    assert_eq!(fields.len(), 7);
}

async fn openai_ignored_inputs() {
    let mut call = base_call("openai");
    call.extra_headers = Some(iter::once(("X-Ignored".to_owned(), "x".to_owned())).collect());
    call.extra_body = Some(object(&[("ignored", Json::Bool(true))]));
    let (outcome, calls) = ran(settings(), &call).await;
    outcome.expect("the provider answered");
    let fields = body(&openai_target("openai", calls).data);
    assert!(!fields.contains_key("ignored"));
    assert_eq!(fields.len(), 3);
    let nulled: CreateFineTuningJobCall = serde_json::from_str(
        r#"{"model":"gpt-4o-mini","training_file":"file_train","hyperparameters":null}"#,
    )
    .expect("the call decodes");
    let (nulled_outcome, nulled_calls) = ran(settings(), &nulled).await;
    nulled_outcome.expect("the provider answered");
    assert_eq!(
        body(&openai_target("openai", nulled_calls).data).get("hyperparameters"),
        Some(&Json::Object(Extra::new()))
    );
}

#[tokio::test]
async fn openai_tuning_body() {
    openai_defaults().await;
    openai_supplied().await;
    openai_ignored_inputs().await;
}

async fn azure_split() {
    let mut call = base_call("azure");
    call.hyperparameters = Some(object(&[
        ("n_epochs", number(2)),
        ("prompt_loss_weight", number(1)),
    ]));
    call.extra_body = Some(object(&[
        ("trainingType", number(9)),
        ("ignored", Json::Bool(true)),
        ("azure_ad_token", text("secret")),
    ]));
    call.params.training_type = Some(number(2));
    let (outcome, calls) = ran(settings(), &call).await;
    outcome.expect("the provider answered");
    let fields = body(&azure_target(calls).data);
    assert_eq!(
        fields.get("hyperparameters"),
        Some(&Json::Object(object(&[("n_epochs", number(2))])))
    );
    let extra_body = match fields.get("extra_body") {
        Some(Json::Object(entries)) => entries.clone(),
        other => panic!("unexpected extra_body {other:?}"),
    };
    assert_eq!(
        extra_body,
        object(&[
            ("trainingType", number(2)),
            ("prompt_loss_weight", number(1))
        ])
    );
    assert!(!extra_body.contains_key("azure_ad_token"));
    assert!(!extra_body.contains_key("ignored"));
}

async fn azure_training_type() {
    let (outcome, calls) = ran(settings(), &base_call("azure")).await;
    outcome.expect("the provider answered");
    assert_eq!(
        body(&azure_target(calls).data).get("extra_body"),
        None,
        "an omitted trainingType is B542's default"
    );
    let mut zero = base_call("azure");
    zero.params.training_type = Some(number(0));
    let (zero_outcome, zero_calls) = ran(settings(), &zero).await;
    zero_outcome.expect("the provider answered");
    assert_eq!(
        body(&azure_target(zero_calls).data).get("extra_body"),
        Some(&Json::Object(object(&[("trainingType", number(0))])))
    );
}

fn prepare(mut existing: Extra, training_type: Option<&Json>, specific: &Extra) -> Extra {
    prepare_azure_extra_body(&mut existing, training_type, specific);
    existing
}

#[tokio::test]
async fn azure_training_parameters() {
    azure_split().await;
    azure_training_type().await;
    let training_type = number(2);
    let specific = object(&[("prompt_loss_weight", number(1))]);
    let existing = object(&[("trainingType", number(9)), ("keep", Json::Bool(true))]);
    assert_eq!(
        prepare(existing, Some(&training_type), &specific),
        object(&[
            ("trainingType", number(2)),
            ("keep", Json::Bool(true)),
            ("prompt_loss_weight", number(1)),
        ])
    );
    let colliding = object(&[("trainingType", number(5))]);
    assert_eq!(
        prepare(Extra::new(), Some(&training_type), &colliding),
        object(&[("trainingType", number(5))])
    );
    assert_eq!(prepare(Extra::new(), None, &Extra::new()), Extra::new());
}

async fn vertex_request() {
    let mut call = base_call("vertex_ai");
    call.model = "gemini-1.5-pro".to_owned();
    call.training_file = "gs://bucket/train.jsonl".to_owned();
    call.hyperparameters = Some(object(&[
        ("n_epochs", number(2)),
        ("adapter_size", number(4)),
    ]));
    call.validation_file = Some("gs://bucket/valid.jsonl".to_owned());
    call.suffix = Some("custom".to_owned());
    call.params.api_base = Some("https://ignored.example/v1".to_owned());
    call.params.vertex_project = Some("request-project".to_owned());
    let (outcome, calls) = ran(settings(), &call).await;
    outcome.expect("the provider answered");
    let target = vertex_target(calls);
    assert_eq!(target.api_base, "https://ignored.example/v1");
    assert_eq!(target.vertex_project.as_deref(), Some("request-project"));
    assert_eq!(
        target.original_hyperparameters,
        object(&[("n_epochs", number(2)), ("adapter_size", number(4))])
    );
    let fields = body(&target.data);
    assert_eq!(fields.get("model"), Some(&text("gemini-1.5-pro")));
    assert_eq!(
        fields.get("training_file"),
        Some(&text("gs://bucket/train.jsonl"))
    );
    assert_eq!(
        fields.get("validation_file"),
        Some(&text("gs://bucket/valid.jsonl"))
    );
    assert_eq!(fields.get("suffix"), Some(&text("custom")));
}

async fn vertex_defaults() {
    let (outcome, calls) = ran(settings(), &base_call("vertex_ai")).await;
    outcome.expect("the provider answered");
    let target = vertex_target(calls);
    assert_eq!(target.api_base, "");
    assert_eq!(target.vertex_project, None);
    assert_eq!(target.vertex_location, None);
}

fn vertex_response() {
    let job = decode_fine_tune_job(
        br#"{"id":"ft_v","object":"fine_tuning.job","created_at":1,"model":"gemini-1.5-pro","organization_id":"","result_files":[],"status":"queued","training_file":"gs://bucket/train.jsonl","hyperparameters":{"n_epochs":2,"adapter_size":4}}"#,
        false,
    )
    .expect("the vertex job decodes");
    let hyperparameters: Hyperparameters = job.hyperparameters;
    let encoded = serde_json::to_string(&hyperparameters).expect("the record encodes");
    assert!(encoded.contains("\"adapter_size\":4"));
}

#[tokio::test]
async fn vertex_tuning_dispatch() {
    vertex_request().await;
    vertex_defaults().await;
    vertex_response();
}

#[derive(Clone, Copy)]
enum Selected {
    OpenAi,
    Azure,
}

fn selection(selected: Selected) -> ResourceSdkSettings {
    let mut settings = settings();
    settings.api_base = Some("https://settings-base.test".to_owned());
    match selected {
        Selected::OpenAi => {
            settings.api_key = Some(SecretString::from("sk-settings".to_owned()));
            settings.openai_key = Some(SecretString::from("sk-openai-key".to_owned()));
            settings.openai_api_key = Some(SecretString::from("sk-openai-api-key".to_owned()));
            settings.organization = Some("org-settings".to_owned());
            settings.openai_organization = Some("org-env".to_owned());
            settings.openai_base_url = Some("https://base-url.test".to_owned());
            settings.openai_api_base = Some("https://api-base.test".to_owned());
        }
        Selected::Azure => {
            settings.api_version = Some("2023-05-01".to_owned());
            settings.api_key = Some(SecretString::from("azure-settings".to_owned()));
            settings.azure_key = Some(SecretString::from("azure-key".to_owned()));
            settings.azure_api_base = Some("https://azure-env-base.test".to_owned());
            settings.azure_api_version = Some("2024-02-01".to_owned());
            settings.azure_openai_api_key = Some(SecretString::from("azure-openai".to_owned()));
            settings.azure_api_key = Some(SecretString::from("azure-api".to_owned()));
        }
    }
    settings
}

async fn openai_request_wins() {
    let mut call = base_call("openai");
    call.params.api_base = Some("https://request.test".to_owned());
    call.params.api_key = Some(SecretString::from("sk-request".to_owned()));
    call.params.organization = Some("org-request".to_owned());
    call.params.api_version = Some("2024-01-01".to_owned());
    let (outcome, calls) = ran(selection(Selected::OpenAi), &call).await;
    outcome.expect("the provider answered");
    let target = openai_target("openai", calls);
    assert_eq!(target.api_base, "https://request.test");
    assert_eq!(target.api_key.as_ref().map(secret), Some("sk-request"));
    assert_eq!(target.organization.as_deref(), Some("org-request"));
    assert_eq!(target.api_version.as_deref(), Some("2024-01-01"));
}

async fn openai_base_fallthrough() {
    let mut settings = selection(Selected::OpenAi);
    settings.api_base = None;
    let (outcome, calls) = ran(settings.clone(), &base_call("openai")).await;
    outcome.expect("the provider answered");
    let target = openai_target("openai", calls);
    assert_eq!(target.api_base, "https://base-url.test");
    assert_eq!(target.api_key.as_ref().map(secret), Some("sk-settings"));
    assert_eq!(target.organization.as_deref(), Some("org-settings"));
    settings.openai_base_url = None;
    let (url_outcome, url_calls) = ran(settings.clone(), &base_call("openai")).await;
    url_outcome.expect("the provider answered");
    assert_eq!(
        openai_target("openai", url_calls).api_base,
        "https://api-base.test"
    );
    settings.openai_api_base = None;
    let (default_outcome, default_calls) = ran(settings, &base_call("openai")).await;
    default_outcome.expect("the provider answered");
    assert_eq!(
        openai_target("openai", default_calls).api_base,
        "https://api.openai.com/v1"
    );
}

async fn openai_key_fallthrough() {
    let mut settings = selection(Selected::OpenAi);
    settings.api_key = Some(SecretString::from(String::new()));
    let (outcome, calls) = ran(settings.clone(), &base_call("openai")).await;
    outcome.expect("the provider answered");
    assert_eq!(
        openai_target("openai", calls).api_key.as_ref().map(secret),
        Some("sk-openai-key")
    );
    settings.openai_key = None;
    let (env_key_outcome, env_key_calls) = ran(settings.clone(), &base_call("openai")).await;
    env_key_outcome.expect("the provider answered");
    assert_eq!(
        openai_target("openai", env_key_calls)
            .api_key
            .as_ref()
            .map(secret),
        Some("sk-openai-api-key")
    );
    settings.organization = None;
    let (org_outcome, org_calls) = ran(settings, &base_call("openai")).await;
    org_outcome.expect("the provider answered");
    assert_eq!(
        openai_target("openai", org_calls).organization.as_deref(),
        Some("org-env")
    );
}

async fn azure_selection() {
    let mut settings = selection(Selected::Azure);
    settings.organization = Some("org-settings".to_owned());
    let (outcome, calls) = ran(settings.clone(), &base_call("azure")).await;
    outcome.expect("the provider answered");
    let target = azure_target(calls);
    assert_eq!(
        target.api_base.as_deref(),
        Some("https://settings-base.test")
    );
    assert_eq!(target.api_version.as_deref(), Some("2023-05-01"));
    assert_eq!(target.api_key.as_ref().map(secret), Some("azure-settings"));
    // LiteLLM hands Azure the request organization alone, so no settings value reaches it
    assert_eq!(target.organization, None);
    let mut request = base_call("azure");
    request.params.organization = Some("org-request".to_owned());
    let (request_outcome, request_calls) = ran(settings, &request).await;
    request_outcome.expect("the provider answered");
    assert_eq!(
        azure_target(request_calls).organization.as_deref(),
        Some("org-request")
    );
}

async fn azure_fallthrough() {
    let mut settings = selection(Selected::Azure);
    settings.api_base = None;
    settings.api_version = None;
    settings.api_key = Some(SecretString::from(String::new()));
    let (outcome, calls) = ran(settings.clone(), &base_call("azure")).await;
    outcome.expect("the provider answered");
    let target = azure_target(calls);
    assert_eq!(
        target.api_base.as_deref(),
        Some("https://azure-env-base.test")
    );
    assert_eq!(target.api_version.as_deref(), Some("2024-02-01"));
    assert_eq!(target.api_key.as_ref().map(secret), Some("azure-key"));
    settings.azure_key = Some(SecretString::from(String::new()));
    let (env_key_outcome, env_key_calls) = ran(settings.clone(), &base_call("azure")).await;
    env_key_outcome.expect("the provider answered");
    assert_eq!(
        azure_target(env_key_calls).api_key.as_ref().map(secret),
        Some("azure-openai")
    );
    settings.azure_openai_api_key = None;
    let (api_key_outcome, api_key_calls) = ran(settings, &base_call("azure")).await;
    api_key_outcome.expect("the provider answered");
    assert_eq!(
        azure_target(api_key_calls).api_key.as_ref().map(secret),
        Some("azure-api")
    );
}

async fn vertex_selection() {
    let mut settings = settings();
    settings.vertex_project = Some("global-project".to_owned());
    settings.vertex_location = Some("global-location".to_owned());
    settings.vertexai_project = Some("env-project".to_owned());
    settings.vertexai_location = Some("env-location".to_owned());
    settings.vertexai_credentials = Some(SecretString::from("env-credentials".to_owned()));
    let mut call = base_call("vertex_ai");
    call.params.vertex_project = Some("request-project".to_owned());
    let (outcome, calls) = ran(settings.clone(), &call).await;
    outcome.expect("the provider answered");
    let target = vertex_target(calls);
    assert_eq!(target.vertex_project.as_deref(), Some("request-project"));
    assert_eq!(target.vertex_location.as_deref(), Some("global-location"));
    assert_eq!(
        target.vertex_credentials.as_ref().map(secret),
        Some("env-credentials")
    );
    call.params.vertex_project = None;
    let (global_outcome, global_calls) = ran(settings, &call).await;
    global_outcome.expect("the provider answered");
    assert_eq!(
        vertex_target(global_calls).vertex_project.as_deref(),
        Some("global-project")
    );
}

fn timeout_call(provider: &str) -> CreateFineTuningJobCall {
    let mut call = base_call(provider);
    call.params.max_retries = Some(4);
    call
}

fn structured() -> StructuredTimeout {
    StructuredTimeout {
        connect: Some(1.0),
        read: Some(2.0),
        write: None,
        pool: None,
    }
}

async fn timeout_defaults() {
    let (outcome, calls) = ran(settings(), &timeout_call("openai")).await;
    outcome.expect("the provider answered");
    let target = openai_target("openai", calls);
    assert_eq!(target.timeout, FineTuningTimeout::Seconds(600.0));
    assert_eq!(target.max_retries, Some(4));
    let mut zero = timeout_call("openai");
    zero.params.timeout = Some(CreateTimeout::Plain(TimeoutValue::Number(0.0)));
    zero.params.request_timeout = Some(7.0_f64);
    let (request_outcome, request_calls) = ran(settings(), &zero).await;
    request_outcome.expect("the provider answered");
    assert_eq!(
        openai_target("openai", request_calls).timeout,
        FineTuningTimeout::Seconds(7.0)
    );
    let mut text = timeout_call("openai");
    text.params.timeout = Some(CreateTimeout::Plain(TimeoutValue::String("7.5".to_owned())));
    let (text_outcome, text_calls) = ran(settings(), &text).await;
    text_outcome.expect("the provider answered");
    assert_eq!(
        openai_target("openai", text_calls).timeout,
        FineTuningTimeout::Seconds(7.5)
    );
}

async fn structured_timeouts() {
    let mut openai = timeout_call("openai");
    openai.params.timeout = Some(CreateTimeout::Structured(structured()));
    let (openai_outcome, openai_calls) = ran(settings(), &openai).await;
    openai_outcome.expect("the provider answered");
    assert_eq!(
        openai_target("openai", openai_calls).timeout,
        FineTuningTimeout::Structured(structured())
    );
    let mut azure = timeout_call("azure");
    azure.params.timeout = Some(CreateTimeout::Structured(structured()));
    let (azure_outcome, azure_calls) = ran(settings(), &azure).await;
    azure_outcome.expect("the provider answered");
    assert_eq!(
        azure_target(azure_calls).timeout,
        FineTuningTimeout::Structured(structured())
    );
    let mut vertex = timeout_call("vertex_ai");
    vertex.params.timeout = Some(CreateTimeout::Structured(structured()));
    let (vertex_outcome, vertex_calls) = ran(settings(), &vertex).await;
    vertex_outcome.expect("the provider answered");
    assert_eq!(
        vertex_target(vertex_calls).timeout,
        FineTuningTimeout::Seconds(2.0)
    );
}

async fn invalid_timeout() {
    let mut invalid = base_call("openai");
    invalid.params.timeout = Some(CreateTimeout::Plain(TimeoutValue::String("bad".to_owned())));
    let (outcome, calls) = ran(settings(), &invalid).await;
    assert!(matches!(
        outcome,
        Err(FineTuningCreateError::Timeout(
            TimeoutError::InvalidNumber { .. }
        ))
    ));
    assert!(calls.is_empty());
}

#[tokio::test]
async fn tuning_credentials_and_timeout() {
    openai_request_wins().await;
    openai_base_fallthrough().await;
    openai_key_fallthrough().await;
    azure_selection().await;
    azure_fallthrough().await;
    vertex_selection().await;
    timeout_defaults().await;
    structured_timeouts().await;
    invalid_timeout().await;
}

#[test]
fn tuning_call_wire_form() {
    let params = CreateParams {
        api_key: Some(SecretString::from("sk-wire".to_owned())),
        vertex_credentials: Some(SecretString::from("cred-wire".to_owned())),
        training_type: Some(number(2)),
        timeout: Some(CreateTimeout::Plain(TimeoutValue::Number(2.5_f64))),
        ..CreateParams::default()
    };
    let wire: Json =
        serde_json::from_str(&serde_json::to_string(&params).expect("the params encode"))
            .expect("the wire form is JSON");
    let Json::Object(fields) = wire else {
        panic!("the wire form is an object");
    };
    assert_eq!(fields.get("api_key"), Some(&text("sk-wire")));
    assert_eq!(fields.get("vertex_credentials"), Some(&text("cred-wire")));
    assert_eq!(fields.get("trainingType"), Some(&number(2)));
    assert_eq!(
        fields.get("timeout"),
        Some(&Json::Number(
            Number::from_f64(2.5_f64).expect("a finite number")
        ))
    );
    let structured_params = CreateParams {
        timeout: Some(CreateTimeout::Structured(structured())),
        ..CreateParams::default()
    };
    let structured_wire: Json = serde_json::from_str(
        &serde_json::to_string(&structured_params).expect("the params encode"),
    )
    .expect("the wire form is JSON");
    let Json::Object(structured_fields) = structured_wire else {
        panic!("the wire form is an object");
    };
    assert_eq!(
        structured_fields.get("timeout"),
        Some(&Json::Number(
            Number::from_f64(2.0_f64).expect("a finite number")
        ))
    );
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("utf-8 log output")
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

fn capture() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

fn unsupported_call() -> CreateFineTuningJobCall {
    let mut call = base_call("anthropic");
    call.is_async = true;
    call.params.api_key = Some(SecretString::from("sk-secret-value".to_owned()));
    call
}

async fn unsupported_error() {
    let (outcome, calls) = ran(settings(), &unsupported_call()).await;
    let error = outcome.expect_err("the provider is unsupported");
    assert_eq!(
        error.to_string(),
        "LiteLLM doesn't support anthropic for 'create_batch'. Only 'openai' is supported."
    );
    assert_eq!(
        error.unsupported_provider(),
        Some(UnsupportedProviderError {
            status: 400,
            error_type: None,
            param: None,
            code: None,
            body: UNSUPPORTED_RESPONSE_BODY,
            method: UNSUPPORTED_REQUEST_METHOD,
            url: UNSUPPORTED_REQUEST_URL,
        })
    );
    assert_eq!(UNSUPPORTED_MODEL, "n/a");
    assert!(calls.is_empty());
}

async fn unsupported_log() {
    let (buffer, _guard) = capture();
    let (outcome, _calls) = ran(settings(), &unsupported_call()).await;
    outcome.expect_err("the provider is unsupported");
    let text = buffer.text();
    let lines: Vec<&str> = text.lines().collect();
    let errors: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| line.starts_with("ERROR "))
        .collect();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].starts_with(
        "ERROR got exception in create_fine_tuning_job=LiteLLM doesn't support anthropic"
    ));
    assert!(lines.iter().any(|line| {
        line.starts_with("DEBUG inside acreate_fine_tuning_job model=gpt-4o-mini and kwargs=")
    }));
    assert!(lines.iter().all(|line| !line.contains("sk-secret-value")));
}

async fn unrepresentable_integration() {
    let mut call = base_call("openai");
    call.integrations = Some(vec![text("slack")]);
    let (outcome, calls) = ran(settings(), &call).await;
    assert!(matches!(outcome, Err(FineTuningCreateError::Data(_))));
    assert!(calls.is_empty());
}

#[tokio::test]
async fn unsupported_tuning_create() {
    unsupported_error().await;
    unsupported_log().await;
    unrepresentable_integration().await;
}

struct Refusing;

impl FineTuningCreateDispatch for Refusing {
    type Error = ScriptFailure;

    fn create(
        &self,
        _target: FineTuningCreateTarget,
    ) -> impl Future<Output = Result<FineTuneJob, ScriptFailure>> + Send {
        async {
            Err(ScriptFailure {
                message: "the provider refused".to_owned(),
            })
        }
    }
}

#[tokio::test]
async fn tuning_dispatch_failure() {
    let (buffer, _guard) = capture();
    let service = FineTuningCreate {
        dispatch: Refusing,
        settings: settings(),
    };
    let outcome = service.create(&base_call("openai")).await;
    assert!(matches!(
        outcome,
        Err(FineTuningCreateError::Dispatch(ScriptFailure { .. }))
    ));
    assert!(
        buffer
            .text()
            .contains("ERROR got exception in create_fine_tuning_job=the provider refused")
    );
}

struct Tasked {
    seen: Arc<Mutex<Vec<Option<Id>>>>,
}

impl FineTuningCreateDispatch for Tasked {
    type Error = ScriptFailure;

    fn create(
        &self,
        _target: FineTuningCreateTarget,
    ) -> impl Future<Output = Result<FineTuneJob, ScriptFailure>> + Send {
        let seen = Arc::clone(&self.seen);
        async move {
            seen.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(try_id());
            Ok(job())
        }
    }
}

#[tokio::test]
async fn tuning_dispatch_runs_in_caller_task() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let local_seen = Arc::clone(&seen);
    let local = LocalSet::new();
    let caller = local
        .run_until(async move {
            spawn_local(async move {
                let caller = try_id().expect("the caller runs inside the local task");
                let service = FineTuningCreate {
                    dispatch: Tasked { seen: local_seen },
                    settings: settings(),
                };
                service
                    .create(&base_call("openai"))
                    .await
                    .expect("the provider answered");
                caller
            })
            .await
            .expect("the local task joins")
        })
        .await;
    let recorded = seen.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(recorded, [Some(caller)]);
}
