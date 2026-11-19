#![cfg(test)]

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::resource_sdk_settings::Settings;
use betterlite_models::decisions::Json;
use betterlite_openai::batches::sdk::TimeoutPhases;
use betterlite_openai::fine_tuning::client::{JobsApi, SdkError};
use betterlite_openai::fine_tuning::job::{
    AutoOr, FineTuneCreate, FineTuneJob, Hyperparameters, JobObject, JobStatus,
};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Number};
use thiserror::Error;

use super::{
    AzureFineTuningCancel, FineTuningCancel, FineTuningCancelCall, FineTuningCancelDispatch,
    FineTuningCancelError, FineTuningCancelTarget, FineTuningCancelValues,
    FineTuningOptionalParams, OpenAiFineTuningCancel, OptionalTimeout,
};
use crate::api::resource_sdk_settings::timeout::{FineTuningTimeout, StructuredTimeout};

// what a provider branch received, owned: the shared values and the settings it resolved
#[derive(Debug, Clone, PartialEq)]
enum Recorded {
    OpenAi {
        values: RecordedValues,
        api_base: String,
        api_key: Option<String>,
        api_version: Option<String>,
        organization: Option<String>,
        has_client: bool,
    },
    Azure {
        values: RecordedValues,
        api_base: Option<String>,
        api_key: Option<String>,
        api_version: Option<String>,
        organization: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
struct RecordedValues {
    is_async: bool,
    job_id: String,
    timeout: FineTuningTimeout,
    max_retries: Option<i64>,
}

impl From<FineTuningCancelValues<'_>> for RecordedValues {
    fn from(values: FineTuningCancelValues<'_>) -> Self {
        Self {
            is_async: values.is_async,
            job_id: values.job_id.to_owned(),
            timeout: *values.timeout,
            max_retries: values.max_retries,
        }
    }
}

fn recorded(target: FineTuningCancelTarget<'_>) -> Recorded {
    match target {
        FineTuningCancelTarget::OpenAi(OpenAiFineTuningCancel {
            values,
            api_base,
            api_key,
            api_version,
            organization,
            client,
        }) => Recorded::OpenAi {
            values: RecordedValues::from(values),
            api_base,
            api_key: api_key.map(secret_text),
            api_version: api_version.map(str::to_owned),
            organization: organization.map(str::to_owned),
            has_client: client.is_some(),
        },
        FineTuningCancelTarget::Azure(AzureFineTuningCancel {
            values,
            api_base,
            api_key,
            api_version,
            organization,
        }) => Recorded::Azure {
            values: RecordedValues::from(values),
            api_base: api_base.map(str::to_owned),
            api_key: api_key.map(secret_text),
            api_version: api_version.map(str::to_owned),
            organization: organization.map(str::to_owned),
        },
    }
}

fn secret_text(secret: &SecretString) -> String {
    secret.expose_secret().to_owned()
}

fn base_settings() -> Settings {
    Settings::default()
}

fn base_params() -> FineTuningOptionalParams {
    FineTuningOptionalParams::default()
}

fn answer() -> FineTuneJob {
    FineTuneJob {
        id: "ft_1".to_owned(),
        created_at: 1_704_067_200,
        error: None,
        fine_tuned_model: None,
        finished_at: None,
        hyperparameters: Hyperparameters {
            batch_size: Some(AutoOr::Value(4_i64)),
            learning_rate_multiplier: None,
            n_epochs: None,
            extra: Map::new(),
        },
        model: "gpt-4o-mini".to_owned(),
        object: JobObject::FineTuningJob,
        organization_id: "org_1".to_owned(),
        result_files: Vec::new(),
        seed: None,
        status: JobStatus::Cancelled,
        trained_tokens: None,
        training_file: "file_1".to_owned(),
        validation_file: None,
        estimated_finish: None,
        integrations: None,
        metadata: None,
        method: None,
        extra: Map::new(),
    }
}

fn call(provider: Option<&str>, params: FineTuningOptionalParams) -> FineTuningCancelCall {
    FineTuningCancelCall {
        fine_tuning_job_id: "ft_1".to_owned(),
        custom_llm_provider: provider.map(str::to_owned),
        extra_headers: None,
        extra_body: None,
        params,
        acancel_fine_tuning_job: None,
        client: None,
    }
}

#[derive(Debug, Clone, Error)]
#[error("{message}")]
struct ScriptFailure {
    message: String,
}

fn unused_client_call() -> SdkError {
    SdkError {
        status: 0,
        error_type: None,
        param: None,
        code: None,
        message: "the supplied client runs no request in this test".to_owned(),
    }
}

// the in-process client cancel_fine_tuning_job hands to its openai branch
struct SuppliedClient;

impl JobsApi for SuppliedClient {
    fn is_async(&self) -> bool {
        false
    }

    fn create(&self, _data: &FineTuneCreate) -> Result<Vec<u8>, SdkError> {
        Err(unused_client_call())
    }

    fn list(&self, _after: Option<&str>, _limit: Option<i64>) -> Result<Vec<u8>, SdkError> {
        Err(unused_client_call())
    }

    fn retrieve(&self, _job_id: &str) -> Result<Vec<u8>, SdkError> {
        Err(unused_client_call())
    }

    fn cancel(&self, _job_id: &str) -> Result<Vec<u8>, SdkError> {
        Err(unused_client_call())
    }
}

struct Scripted {
    calls: Arc<Mutex<Vec<Recorded>>>,
    reply: Result<FineTuneJob, ScriptFailure>,
}

impl FineTuningCancelDispatch for Scripted {
    type Error = ScriptFailure;

    fn cancel(
        &self,
        target: FineTuningCancelTarget<'_>,
    ) -> impl Future<Output = Result<FineTuneJob, ScriptFailure>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded(target));
        let reply = self.reply.clone();
        async move { reply }
    }
}

fn scripted() -> (Scripted, Arc<Mutex<Vec<Recorded>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let dispatch = Scripted {
        calls: Arc::clone(&calls),
        reply: Ok(answer()),
    };
    (dispatch, calls)
}

fn recorded_calls(calls: &Arc<Mutex<Vec<Recorded>>>) -> Vec<Recorded> {
    calls.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

// the one call a provider branch received from a run that answered
async fn first_call(settings: Settings, call: FineTuningCancelCall) -> Recorded {
    let (dispatch, calls) = scripted();
    let service = FineTuningCancel { dispatch, settings };
    service.run(&call).await.expect("the provider answered");
    recorded_calls(&calls).pop().expect("one recorded call")
}

fn values(entry: &Recorded) -> &RecordedValues {
    match entry {
        Recorded::OpenAi { values, .. } | Recorded::Azure { values, .. } => values,
    }
}

fn expected_values() -> RecordedValues {
    RecordedValues {
        is_async: false,
        job_id: "ft_1".to_owned(),
        timeout: FineTuningTimeout::Seconds(600.0_f64),
        max_retries: None,
    }
}

// the timeout a run resolved for the branch, whatever the branch is
async fn resolved_timeout(params: FineTuningOptionalParams) -> FineTuningTimeout {
    let entry = first_call(base_settings(), call(Some("openai"), params)).await;
    values(&entry).timeout
}

#[rstest]
#[case::openai("openai")]
#[case::azure("azure")]
#[tokio::test]
async fn cancel_reaches_the_declared_provider_branch(#[case] provider: &str) {
    let entry = first_call(base_settings(), call(Some(provider), base_params())).await;
    let branch = match provider {
        "openai" => matches!(&entry, Recorded::OpenAi { .. }),
        _ => matches!(&entry, Recorded::Azure { .. }),
    };
    assert!(branch, "{provider} takes its own branch");
    assert_eq!(values(&entry), &expected_values());
}

#[tokio::test]
async fn cancel_defaults_to_openai_and_keeps_a_null_provider_unsupported() {
    let mut defaulted = call(Some("openai"), base_params());
    defaulted.custom_llm_provider = super::default_provider();
    let entry = first_call(base_settings(), defaulted).await;
    assert!(matches!(&entry, Recorded::OpenAi { .. }));

    let (dispatch, calls) = scripted();
    let service = FineTuningCancel {
        dispatch,
        settings: base_settings(),
    };
    let error = service
        .run(&call(None, base_params()))
        .await
        .expect_err("a null provider is unsupported");
    assert!(matches!(
        error,
        FineTuningCancelError::Unsupported { ref provider } if provider == "None"
    ));
    assert!(recorded_calls(&calls).is_empty());
}

#[rstest]
#[case::openai("openai")]
#[case::azure("azure")]
#[tokio::test]
async fn cancel_prefers_call_parameters_over_settings(#[case] provider: &str) {
    let mut settings = base_settings();
    settings.api_base = Some("https://settings.example".to_owned());
    settings.api_key = Some(SecretString::from("sk-settings".to_owned()));
    settings.organization = Some("org-settings".to_owned());
    settings.api_version = Some("2024-01-01".to_owned());
    let mut params = base_params();
    params.api_base = Some("https://param.example".to_owned());
    params.api_key = Some(SecretString::from("sk-param".to_owned()));
    params.organization = Some("org-param".to_owned());
    params.api_version = Some("2024-02-02".to_owned());
    let entry = first_call(settings, call(Some(provider), params)).await;
    match &entry {
        Recorded::OpenAi {
            api_base,
            api_key,
            api_version,
            organization,
            ..
        } => {
            assert_eq!(api_base, "https://param.example");
            assert_eq!(api_key.as_deref(), Some("sk-param"));
            assert_eq!(api_version.as_deref(), Some("2024-02-02"));
            assert_eq!(organization.as_deref(), Some("org-param"));
        }
        Recorded::Azure {
            api_base,
            api_key,
            api_version,
            organization,
            ..
        } => {
            assert_eq!(api_base.as_deref(), Some("https://param.example"));
            assert_eq!(api_key.as_deref(), Some("sk-param"));
            assert_eq!(api_version.as_deref(), Some("2024-02-02"));
            assert_eq!(organization.as_deref(), Some("org-param"));
        }
    }
}

#[tokio::test]
async fn cancel_reads_the_proxy_wide_settings_the_environment_filled() {
    let settings = Settings {
        api_base: Some("https://api-base.example".to_owned()),
        openai_key: Some(SecretString::from("sk-openai-key".to_owned())),
        openai_organization: Some("org-openai".to_owned()),
        ..base_settings()
    };
    let entry = first_call(settings, call(Some("openai"), base_params())).await;
    let Recorded::OpenAi {
        api_base,
        api_key,
        organization,
        ..
    } = &entry
    else {
        panic!("openai takes its branch");
    };
    assert_eq!(api_base, "https://api-base.example");
    assert_eq!(api_key.as_deref(), Some("sk-openai-key"));
    assert_eq!(organization.as_deref(), Some("org-openai"));
}

#[tokio::test]
async fn cancel_falls_back_to_the_openai_default_base() {
    let entry = first_call(base_settings(), call(Some("openai"), base_params())).await;
    let Recorded::OpenAi { api_base, .. } = &entry else {
        panic!("openai takes its branch");
    };
    assert_eq!(api_base, "https://api.openai.com/v1");
}

#[rstest]
#[case::azure_openai_key(Some("azure-openai".to_owned()), None, Some("azure-openai"))]
#[case::azure_api_key(None, Some("azure-key".to_owned()), Some("azure-key"))]
#[case::azure_configured_key(Some("azure-openai".to_owned()), Some("azure-key".to_owned()), Some("azure-openai"))]
#[case::azure_key_unset(None, None, None)]
#[tokio::test]
async fn cancel_reads_the_azure_key_chain(
    #[case] openai_key: Option<String>,
    #[case] api_key: Option<String>,
    #[case] expected: Option<&str>,
) {
    let settings = Settings {
        azure_openai_api_key: openai_key.map(SecretString::from),
        azure_api_key: api_key.map(SecretString::from),
        ..base_settings()
    };
    let entry = first_call(settings, call(Some("azure"), base_params())).await;
    let Recorded::Azure { api_key, .. } = &entry else {
        panic!("azure takes its branch");
    };
    assert_eq!(api_key.as_deref(), expected);
}

#[rstest]
#[case::configured(None, None, 600.0_f64)]
#[case::zero_timeout(Some(0.0_f64), None, 600.0_f64)]
#[case::zero_with_request_timeout(Some(0.0_f64), Some(5.0_f64), 5.0_f64)]
#[case::request_timeout_only(None, Some(5.0_f64), 5.0_f64)]
#[case::seconds(Some(7.0_f64), None, 7.0_f64)]
#[case::zero_with_zero_request_timeout(Some(0.0_f64), Some(0.0_f64), 600.0_f64)]
#[tokio::test]
async fn cancel_resolves_the_timeout_defaults(
    #[case] timeout: Option<f64>,
    #[case] request_timeout: Option<f64>,
    #[case] expected: f64,
) {
    let mut params = base_params();
    params.timeout = timeout.map(OptionalTimeout::Seconds);
    params.request_timeout = request_timeout;
    assert_eq!(
        resolved_timeout(params).await,
        FineTuningTimeout::Seconds(expected)
    );
}

#[rstest]
#[case::seconds("7.5", FineTuningTimeout::Seconds(7.5_f64))]
#[case::underscored("1_000", FineTuningTimeout::Seconds(1_000.0_f64))]
#[case::padded(" 12 ", FineTuningTimeout::Seconds(12.0_f64))]
#[tokio::test]
async fn cancel_parses_a_timeout_written_as_text(
    #[case] text: &str,
    #[case] expected: FineTuningTimeout,
) {
    let mut params = base_params();
    params.timeout = Some(OptionalTimeout::Text(text.to_owned()));
    assert_eq!(resolved_timeout(params).await, expected);
}

#[tokio::test]
async fn cancel_rejects_a_timeout_that_is_not_a_number() {
    let mut params = base_params();
    params.timeout = Some(OptionalTimeout::Text("bad".to_owned()));
    let (dispatch, calls) = scripted();
    let service = FineTuningCancel {
        dispatch,
        settings: base_settings(),
    };
    let error = service
        .run(&call(Some("openai"), params))
        .await
        .expect_err("the timeout does not parse");
    assert!(matches!(error, FineTuningCancelError::Timeout(_)));
    assert!(recorded_calls(&calls).is_empty());
}

#[tokio::test]
async fn cancel_fails_a_bad_timeout_before_an_unsupported_provider() {
    let mut params = base_params();
    params.timeout = Some(OptionalTimeout::Text("bad".to_owned()));
    let (dispatch, calls) = scripted();
    let service = FineTuningCancel {
        dispatch,
        settings: base_settings(),
    };
    let error = service
        .run(&call(Some("vertex_ai"), params))
        .await
        .expect_err("the timeout does not parse");
    assert!(matches!(error, FineTuningCancelError::Timeout(_)));
    assert!(recorded_calls(&calls).is_empty());
}

const STRUCTURED: StructuredTimeout = StructuredTimeout {
    connect: Some(1.0_f64),
    read: Some(30.0_f64),
    write: Some(2.0_f64),
    pool: Some(3.0_f64),
};

#[rstest]
#[case::openai(Some("openai"), FineTuningTimeout::Structured(STRUCTURED))]
#[case::azure(Some("azure"), FineTuningTimeout::Structured(STRUCTURED))]
#[case::vertex_ai(Some("vertex_ai"), FineTuningTimeout::Seconds(30.0_f64))]
#[tokio::test]
async fn cancel_keeps_the_structured_timeout_only_where_the_provider_declares_it(
    #[case] provider: Option<&str>,
    #[case] expected: FineTuningTimeout,
) {
    let mut params = base_params();
    params.timeout = Some(OptionalTimeout::Phases(TimeoutPhases {
        connect: Some(1.0_f64),
        read: Some(30.0_f64),
        write: Some(2.0_f64),
        pool: Some(3.0_f64),
    }));
    let entry = first_call(base_settings(), call(provider, params)).await;
    assert_eq!(values(&entry).timeout, expected);
}

#[tokio::test]
async fn cancel_defaults_the_phases_read_timeout_when_the_provider_drops_them() {
    let mut params = base_params();
    params.timeout = Some(OptionalTimeout::Phases(TimeoutPhases {
        connect: Some(1.0_f64),
        read: None,
        write: None,
        pool: None,
    }));
    let entry = first_call(base_settings(), call(Some("vertex_ai"), params)).await;
    assert_eq!(
        values(&entry).timeout,
        FineTuningTimeout::Seconds(600.0_f64)
    );
}

#[rstest]
#[case::true_flag(Some(Json::Bool(true)), true)]
#[case::one(Some(Json::Number(Number::from(1_u8))), false)]
#[case::text(Some(Json::Text("true".to_owned())), false)]
#[case::null(Some(Json::Null), false)]
#[case::absent(None, false)]
#[tokio::test]
async fn cancel_reads_a_true_only_async_flag(#[case] flag: Option<Json>, #[case] expected: bool) {
    let mut request = call(Some("openai"), base_params());
    request.acancel_fine_tuning_job = flag;
    let entry = first_call(base_settings(), request).await;
    assert_eq!(values(&entry).is_async, expected);
}

#[tokio::test]
async fn cancel_hands_the_supplied_client_to_openai_only() {
    let mut request = call(Some("openai"), base_params());
    request.client = Some(Box::new(SuppliedClient));
    let entry = first_call(base_settings(), request).await;
    let Recorded::OpenAi { has_client, .. } = &entry else {
        panic!("openai takes its branch");
    };
    assert!(has_client);

    // the azure branch has no client field at all, so a supplied client is dropped
    let mut request = call(Some("azure"), base_params());
    request.client = Some(Box::new(SuppliedClient));
    let entry = first_call(base_settings(), request).await;
    assert!(matches!(&entry, Recorded::Azure { .. }));
}

#[tokio::test]
async fn cancel_passes_max_retries_to_the_branch() {
    let mut params = base_params();
    params.max_retries = Some(3);
    let entry = first_call(base_settings(), call(Some("openai"), params)).await;
    assert_eq!(values(&entry).max_retries, Some(3));
}

#[tokio::test]
async fn cancel_keeps_the_public_extras_and_the_azure_token_out_of_the_branch() {
    let mut request = call(Some("azure"), base_params());
    request.extra_headers = Some(IndexMap::from([("x-public".to_owned(), "1".to_owned())]));
    request.extra_body = Some(IndexMap::from([(
        "azure_ad_token".to_owned(),
        "ad-token".to_owned(),
    )]));
    request.params.azure_ad_token = Some(SecretString::from("ad-token".to_owned()));
    let entry = first_call(base_settings(), request).await;
    // the recorded branch carries no header, body or token: those fields do not exist on it
    let Recorded::Azure {
        api_base,
        api_key,
        api_version,
        organization,
        ..
    } = &entry
    else {
        panic!("azure takes its branch");
    };
    assert_eq!(api_base, &None);
    assert_eq!(api_key, &None);
    assert_eq!(api_version, &None);
    assert_eq!(organization, &None);
}

#[rstest]
#[case::vertex_ai("vertex_ai")]
#[case::other("anthropic")]
#[case::empty("")]
#[tokio::test]
async fn cancel_rejects_a_provider_it_has_no_branch_for(#[case] provider: &str) {
    let (dispatch, calls) = scripted();
    let service = FineTuningCancel {
        dispatch,
        settings: base_settings(),
    };
    let error = service
        .run(&call(Some(provider), base_params()))
        .await
        .expect_err("the provider is unsupported");
    let FineTuningCancelError::Unsupported { provider: named } = &error else {
        panic!("the provider is unsupported");
    };
    assert_eq!(named, provider);
    let fields = error.error_fields().expect("a bad request carries fields");
    assert_eq!(fields.status, 400);
    assert_eq!(fields.error_type, None);
    assert_eq!(fields.param, None);
    assert_eq!(fields.code, None);
    assert!(recorded_calls(&calls).is_empty());
}

#[test]
fn cancel_names_the_source_message_of_an_unsupported_provider() {
    let error = FineTuningCancelError::<ScriptFailure>::Unsupported {
        provider: "vertex_ai".to_owned(),
    };
    assert_eq!(
        error.to_string(),
        "LiteLLM doesn't support vertex_ai for 'create_batch'. Only 'openai' is supported."
    );
}

#[test]
fn cancel_reports_no_error_fields_for_a_dispatch_failure() {
    let error = FineTuningCancelError::Dispatch(ScriptFailure {
        message: "the provider failed".to_owned(),
    });
    assert!(error.error_fields().is_none());
}
