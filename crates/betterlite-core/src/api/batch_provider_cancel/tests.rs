#![cfg(test)]

mod cancel;
mod decoding;
mod invoke;

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::batch_provider_cancel::Settings;
use betterlite_openai::api::batch_records::BatchIdRequest;
use betterlite_openai::batches::records::{BatchObject, BatchObjectKind, BatchStatus};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use super::async_invoke::{self, AsyncInvokeOutput, AsyncInvokeS3Output, AsyncInvokeStatus};
use super::connection::CancelParams;
use super::service::BatchProviderCancel;
use super::{
    BatchCancelDispatch, BatchProviderCancelInput, BatchProviderCancelResult, CancelBatchCall,
    CancelCall, CancelProvider, CancelTarget, CancelValues,
};
use crate::api::api_boundaries::ApiLimits;
use crate::api::resource_sdk_settings::timeout::FineTuningTimeout;

const SECOND: i64 = 1_704_067_200;

// what a provider branch received, owned: the shared values and the settings it resolved
#[derive(Debug, Clone, PartialEq)]
enum Recorded {
    OpenAi {
        values: RecordedValues,
        api_base: String,
        api_key: Option<String>,
        organization: Option<String>,
    },
    Azure {
        values: RecordedValues,
        api_base: Option<String>,
        api_key: Option<String>,
        api_version: Option<String>,
        azure_ad_token: Option<String>,
    },
    Vertex {
        values: RecordedValues,
        api_base: Option<String>,
        vertex_project: Option<String>,
        vertex_location: Option<String>,
        vertex_credentials: Option<String>,
    },
    Xai {
        values: RecordedValues,
        api_base: Option<String>,
        api_key: Option<String>,
    },
    Bedrock {
        values: RecordedValues,
        aws_region_name: Option<String>,
        aws_access_key_id: Option<String>,
        aws_session_token: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
struct RecordedValues {
    is_async: bool,
    batch_id: String,
    request: BatchIdRequest,
    timeout: FineTuningTimeout,
    max_retries: Option<i64>,
}

impl From<CancelValues<'_>> for RecordedValues {
    fn from(values: CancelValues<'_>) -> Self {
        Self {
            is_async: values.is_async,
            batch_id: values.batch_id.to_owned(),
            request: values.request.clone(),
            timeout: *values.timeout,
            max_retries: values.max_retries,
        }
    }
}

fn recorded(target: CancelTarget<'_>) -> Recorded {
    match target {
        CancelTarget::OpenAi(call) => Recorded::OpenAi {
            values: RecordedValues::from(call.values),
            api_base: call.api_base,
            api_key: call.api_key.map(secret_text),
            organization: call.organization.map(str::to_owned),
        },
        CancelTarget::Azure(call) => Recorded::Azure {
            values: RecordedValues::from(call.values),
            api_base: call.api_base.map(str::to_owned),
            api_key: call.api_key.map(secret_text),
            api_version: call.api_version.map(str::to_owned),
            azure_ad_token: call.azure_ad_token.map(secret_text),
        },
        CancelTarget::Vertex(call) => Recorded::Vertex {
            values: RecordedValues::from(call.values),
            api_base: call.api_base.map(str::to_owned),
            vertex_project: call.vertex_project.map(str::to_owned),
            vertex_location: call.vertex_location.map(str::to_owned),
            vertex_credentials: call.vertex_credentials.map(secret_text),
        },
        CancelTarget::Xai(call) => Recorded::Xai {
            values: RecordedValues::from(call.values),
            api_base: call.api_base.map(str::to_owned),
            api_key: call.api_key.map(secret_text),
        },
        CancelTarget::Bedrock(call) => Recorded::Bedrock {
            values: RecordedValues::from(call.values),
            aws_region_name: call.params.aws_region_name.clone(),
            aws_access_key_id: call.params.aws_access_key_id.clone(),
            aws_session_token: call.params.aws_session_token.as_ref().map(secret_text),
        },
    }
}

fn branch(entry: &Recorded) -> &'static str {
    match entry {
        Recorded::OpenAi { .. } => "openai",
        Recorded::Azure { .. } => "azure",
        Recorded::Vertex { .. } => "vertex",
        Recorded::Xai { .. } => "xai",
        Recorded::Bedrock { .. } => "bedrock",
    }
}

fn values(entry: &Recorded) -> &RecordedValues {
    match entry {
        Recorded::OpenAi { values, .. }
        | Recorded::Azure { values, .. }
        | Recorded::Vertex { values, .. }
        | Recorded::Xai { values, .. }
        | Recorded::Bedrock { values, .. } => values,
    }
}

fn secret_text(secret: &SecretString) -> String {
    secret.expose_secret().to_owned()
}

fn base_settings() -> Settings {
    Settings {
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

fn base_params() -> CancelParams {
    CancelParams {
        api_base: None,
        api_key: None,
        api_version: None,
        organization: None,
        vertex_project: None,
        vertex_location: None,
        vertex_credentials: None,
        azure_ad_token: None,
        aws_region_name: None,
        aws_access_key_id: None,
        aws_secret_access_key: None,
        aws_session_token: None,
        timeout: None,
        request_timeout: None,
        max_retries: None,
    }
}

fn answer(status: BatchStatus) -> BatchObject {
    BatchObject {
        id: "batch_1".to_owned(),
        completion_window: "24h".to_owned(),
        created_at: SECOND,
        endpoint: "/v1/chat/completions".to_owned(),
        input_file_id: "file_1".to_owned(),
        object: BatchObjectKind::Batch,
        status,
        cancelled_at: None,
        cancelling_at: Some(SECOND),
        completed_at: None,
        error_file_id: None,
        errors: None,
        expired_at: None,
        expires_at: None,
        failed_at: None,
        finalizing_at: None,
        in_progress_at: None,
        metadata: None,
        model: None,
        output_file_id: None,
        request_counts: None,
        usage: None,
        provider_fields: <_>::default(),
    }
}

fn expected_values() -> RecordedValues {
    RecordedValues {
        is_async: false,
        batch_id: "batch_1".to_owned(),
        request: BatchIdRequest {
            batch_id: Some("batch_1".to_owned()),
            extra_headers: None,
            extra_body: None,
            timeout: None,
        },
        timeout: FineTuningTimeout::Seconds(600.0),
        max_retries: None,
    }
}

#[derive(Debug, Clone, Error)]
#[error("{message}")]
struct ScriptFailure {
    message: String,
}

struct Scripted {
    calls: Arc<Mutex<Vec<Recorded>>>,
    reply: Result<BatchObject, ScriptFailure>,
}

impl BatchCancelDispatch for Scripted {
    type Error = ScriptFailure;

    fn cancel(
        &self,
        target: CancelTarget<'_>,
    ) -> impl Future<Output = Result<BatchObject, ScriptFailure>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded(target));
        let reply = self.reply.clone();
        async move { reply }
    }
}

fn scripted(status: BatchStatus) -> (Scripted, Arc<Mutex<Vec<Recorded>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let dispatch = Scripted {
        calls: Arc::clone(&calls),
        reply: Ok(answer(status)),
    };
    (dispatch, calls)
}

fn service(settings: Settings, dispatch: Scripted) -> BatchProviderCancel<Scripted> {
    BatchProviderCancel {
        clock: Arc::new(|| SECOND),
        dispatch,
        limits: ApiLimits::default(),
        settings,
    }
}

fn recorded_calls(calls: &Arc<Mutex<Vec<Recorded>>>) -> Vec<Recorded> {
    calls.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

fn request(provider: &str, params: CancelParams, call: CancelCall) -> BatchProviderCancelInput {
    BatchProviderCancelInput::Cancel(Box::new(CancelBatchCall {
        batch_id: "batch_1".to_owned(),
        model: None,
        provider: CancelProvider(provider.to_owned()),
        extra_headers: None,
        extra_body: None,
        params,
        call,
    }))
}

// the one call a provider branch received from a run that answered
async fn first_call(settings: Settings, provider: &str, params: CancelParams) -> Recorded {
    let (dispatch, calls) = scripted(BatchStatus::Cancelling);
    let service = service(settings, dispatch);
    service
        .run(&request(provider, params, CancelCall::Sync))
        .await
        .expect("the provider answered");
    recorded_calls(&calls).pop().expect("one recorded call")
}

// every declared provider reaches its branch with the shared values of the call
#[rstest]
#[case::openai("openai", "openai")]
#[case::hosted_vllm("hosted_vllm", "openai")]
#[case::litellm_proxy("litellm_proxy", "openai")]
#[case::azure("azure", "azure")]
#[case::vertex_ai("vertex_ai", "vertex")]
#[case::xai("xai", "xai")]
#[tokio::test]
async fn batch_cancel(#[case] provider: &str, #[case] expected: &str) {
    let mut settings = base_settings();
    settings.api_key = Some(SecretString::from("sk-configured".to_owned()));
    let entry = first_call(settings, provider, base_params()).await;
    assert_eq!(branch(&entry), expected);
    assert_eq!(values(&entry), &expected_values());
}

fn status_response(raw: &str) -> AsyncInvokeStatus {
    AsyncInvokeStatus {
        invocation_arn: "arn:aws:bedrock:us-east-1:1:async-invoke/abc".to_owned(),
        model_arn: "arn:aws:bedrock:us-east-1::foundation-model/marengo".to_owned(),
        status: raw.to_owned(),
        failure_message: None,
        output_data_config: None,
        submit_time: Some("2024-01-01T00:00:00Z".to_owned()),
        last_modified_time: Some("2024-01-01T00:01:00Z".to_owned()),
        end_time: Some("2024-01-01T00:02:00Z".to_owned()),
    }
}

fn async_result(status: &AsyncInvokeStatus) -> BatchProviderCancelResult {
    async_invoke::normalize(status, SECOND, &ApiLimits::default()).expect("normalized")
}

#[rstest]
#[case::completed("Completed", BatchStatus::Completed, 1, 0)]
#[case::lowercase_completed("completed", BatchStatus::Completed, 1, 0)]
#[case::uppercase_completed("COMPLETED", BatchStatus::Completed, 1, 0)]
#[case::in_progress("InProgress", BatchStatus::InProgress, 0, 0)]
#[case::in_progress_snake("in_progress", BatchStatus::InProgress, 0, 0)]
#[case::validating("Validating", BatchStatus::Failed, 0, 1)]
fn async_invoke_completed(
    #[case] raw: &str,
    #[case] expected: BatchStatus,
    #[case] completed: i64,
    #[case] failed: i64,
) {
    let result = async_result(&status_response(raw));
    let batch = result.batch;
    assert_eq!(batch.status, expected);
    assert_eq!(batch.object, BatchObjectKind::Batch);
    assert_eq!(batch.endpoint, "/v1/embeddings");
    assert_eq!(batch.completion_window, "24h");
    assert_eq!(batch.input_file_id, "");
    assert_eq!(batch.created_at, SECOND);
    assert_eq!(batch.id, "arn:aws:bedrock:us-east-1:1:async-invoke/abc");
    let counts = batch.request_counts.expect("counts");
    assert_eq!(counts.total, 1);
    assert_eq!(counts.completed, completed);
    assert_eq!(counts.failed, failed);
    assert_eq!(result.model, None);
}

#[rstest]
#[case::failed("Failed")]
#[case::lowercase_failed("failed")]
#[case::stopped("Stopped")]
#[case::partially_completed("PartiallyCompleted")]
#[case::unknown("Exploded")]
#[case::empty("")]
fn async_invoke_failed(#[case] raw: &str) {
    let mut status = status_response(raw);
    status.failure_message = Some("the job stopped".to_owned());
    let batch = async_result(&status).batch;
    assert_eq!(batch.status, BatchStatus::Failed);
    let counts = batch.request_counts.expect("counts");
    assert_eq!(counts.failed, 1);
    assert_eq!(counts.completed, 0);
    let metadata = batch.metadata.expect("metadata");
    assert_eq!(
        metadata.get("failure_message").map(String::as_str),
        Some("the job stopped")
    );
}

#[rstest]
#[case::s3_uri(
    Some(AsyncInvokeOutput {
        s3_output_data_config: Some(AsyncInvokeS3Output {
            s3_uri: Some("s3://bucket/output.jsonl".to_owned()),
        }),
    }),
    "s3://bucket/output.jsonl"
)]
#[case::missing_output_config(None, "")]
#[case::missing_s3_config(
    Some(AsyncInvokeOutput {
        s3_output_data_config: None,
    }),
    ""
)]
#[case::missing_s3_uri(
    Some(AsyncInvokeOutput {
        s3_output_data_config: Some(AsyncInvokeS3Output { s3_uri: None }),
    }),
    ""
)]
fn async_invoke_output_location(
    #[case] output_data_config: Option<AsyncInvokeOutput>,
    #[case] expected: &str,
) {
    let mut status = status_response("Completed");
    status.output_data_config = output_data_config;
    let batch = async_result(&status).batch;
    let metadata = batch.metadata.expect("metadata");
    assert_eq!(
        metadata.get("output_file_id").map(String::as_str),
        Some(expected)
    );
    assert_eq!(
        metadata.get("model_arn").map(String::as_str),
        Some("arn:aws:bedrock:us-east-1::foundation-model/marengo")
    );
    assert_eq!(batch.output_file_id, None);
}
