use super::azure;
use super::call::Run;
use super::client::{self, HttpBuilder};
use super::outcome::{Outcome, RunOutcome, job_view};
use super::vertex;
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::Failure;
use crate::sdk_http::result::kinds::SdkResult;
use betterlite_azure::fine_tuning::error::AzureFineTuneError;
use betterlite_core::api::fine_tuning_create::connection::CreateParams;
use betterlite_core::api::fine_tuning_create::service::FineTuningCreate;
use betterlite_core::api::fine_tuning_create::{
    CreateFineTuningJobCall, FineTuningCreateDispatch, FineTuningCreateError,
    FineTuningCreateTarget, OpenAiCreateCall,
};
use betterlite_core::api::resource_sdk_settings::ResourceSdkSettings;
use betterlite_core::api::resource_sdk_settings::timeout::FineTuningTimeout;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::fine_tuning::client::{ClientSettings, SdkError};
use betterlite_openai::fine_tuning::error::FineTuneError;
use betterlite_openai::fine_tuning::job::FineTuneJob;
use betterlite_openai::fine_tuning::{Connection, JobRequest, request_fine_tuning_job};
use betterlite_vertex::vertex_fine_tuning::execution::TuningExecutionError;
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SendError {
    #[error(transparent)]
    OpenAi(#[from] FineTuneError),
    #[error(transparent)]
    Azure(#[from] AzureFineTuneError),
    #[error(transparent)]
    Vertex(#[from] TuningExecutionError),
    #[error("the oracle client could not be built: {message}")]
    Setup { message: String },
}
pub async fn run(
    runs: &[Run],
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let mut outcomes = Vec::with_capacity(runs.len());
    for run in runs {
        outcomes.push(one(run, api_base, env).await);
    }
    Ok(SdkResult::FineTuningCreate {
        value: Box::new(Outcome { runs: outcomes }),
    })
}
async fn one(run: &Run, api_base: &str, env: &BTreeMap<String, String>) -> RunOutcome {
    let urls = Arc::new(Mutex::new(Vec::new()));
    let service = FineTuningCreate {
        dispatch: Dispatch {
            urls: Arc::clone(&urls),
        },
        settings: settings(env),
    };
    let outcome = service.create(&create_call(run, api_base)).await;
    let request_url = urls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
        .cloned();
    match outcome {
        Ok(job) => match job_view(&job) {
            Ok(view) => RunOutcome {
                job: Some(view),
                failure: None,
                request_url,
            },
            Err(error) => failed(exception_failure(&error.to_string()), request_url),
        },
        Err(error) => failed(project(&error), request_url),
    }
}
const fn failed(failure: Failure, request_url: Option<String>) -> RunOutcome {
    RunOutcome {
        job: None,
        failure: Some(failure),
        request_url,
    }
}
fn exception_failure(message: &str) -> Failure {
    Failure {
        class: "Exception".to_owned(),
        status: 0,
        kind: Some(None),
        param: Some(None),
        code: Some(None),
        message: message.to_owned(),
    }
}
fn project(error: &FineTuningCreateError<SendError>) -> Failure {
    match error {
        FineTuningCreateError::Unsupported { .. } => Failure {
            class: "BadRequestError".to_owned(),
            status: 400,
            kind: Some(None),
            param: Some(None),
            code: Some(None),
            message: format!("litellm.BadRequestError: {error}"),
        },
        FineTuningCreateError::Dispatch(SendError::OpenAi(FineTuneError::Sdk { source }))
        | FineTuningCreateError::Dispatch(SendError::Azure(AzureFineTuneError::Sdk { source })) => {
            sdk_failure(source)
        }
        FineTuningCreateError::Timeout(_)
        | FineTuningCreateError::Data(_)
        | FineTuningCreateError::Dispatch(_) => exception_failure(&error.to_string()),
    }
}
fn sdk_failure(error: &SdkError) -> Failure {
    Failure {
        class: client::error_class(error.status).to_owned(),
        status: error.status,
        kind: Some(error.error_type.clone()),
        param: Some(error.param.clone()),
        code: Some(error.code.clone()),
        message: error.message.clone(),
    }
}
#[derive(Default)]
struct Dispatch {
    urls: Arc<Mutex<Vec<String>>>,
}
impl FineTuningCreateDispatch for Dispatch {
    type Error = SendError;
    fn create(
        &self,
        target: FineTuningCreateTarget,
    ) -> impl Future<Output = Result<FineTuneJob, SendError>> + Send {
        let urls = Arc::clone(&self.urls);
        async move {
            match target {
                FineTuningCreateTarget::OpenAi(call) => openai(call),
                FineTuningCreateTarget::Azure(call) => azure::create(call),
                FineTuningCreateTarget::Vertex(call) => vertex::create(call, urls).await,
            }
        }
    }
}
fn openai(call: OpenAiCreateCall) -> Result<FineTuneJob, SendError> {
    let OpenAiCreateCall {
        is_async,
        data,
        api_base,
        api_key,
        api_version,
        organization,
        timeout,
        max_retries,
    } = call;
    let connection = Connection {
        settings: ClientSettings {
            api_key,
            api_base: Some(api_base),
            api_version,
            timeout: seconds(timeout),
            max_retries,
            organization,
        },
        litellm_params: None,
        client: None,
        is_async,
        builder: &HttpBuilder,
    };
    request_fine_tuning_job(connection, &JobRequest::Create(&data)).map_err(SendError::from)
}
pub fn seconds(timeout: FineTuningTimeout) -> Option<f64> {
    match timeout {
        FineTuningTimeout::Seconds(value) => Some(value),
        FineTuningTimeout::Structured(found) => {
            found.read.or(found.connect).or(found.write).or(found.pool)
        }
    }
}
// the case env is the only settings source here: every chain falls back to it after the request values
fn settings(env: &BTreeMap<String, String>) -> ResourceSdkSettings {
    let secret = |name: &str| {
        env.get(name)
            .filter(|text| !text.is_empty())
            .map(|text| SecretString::from(text.clone()))
    };
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
        openai_base_url: env.get("OPENAI_BASE_URL").cloned(),
        openai_api_base: env.get("OPENAI_API_BASE").cloned(),
        openai_api_key: secret("OPENAI_API_KEY"),
        openai_organization: env.get("OPENAI_ORGANIZATION").cloned(),
        azure_api_base: env.get("AZURE_API_BASE").cloned(),
        azure_api_version: env.get("AZURE_API_VERSION").cloned(),
        azure_openai_api_key: secret("AZURE_OPENAI_API_KEY"),
        azure_api_key: secret("AZURE_API_KEY"),
        vertexai_project: env.get("VERTEXAI_PROJECT").cloned(),
        vertexai_location: env.get("VERTEXAI_LOCATION").cloned(),
        vertexai_credentials: secret("VERTEXAI_CREDENTIALS"),
    }
}
fn create_call(run: &Run, api_base: &str) -> CreateFineTuningJobCall {
    CreateFineTuningJobCall {
        model: run.model.clone(),
        training_file: run.training_file.clone(),
        hyperparameters: run
            .hyperparameters
            .as_ref()
            .and_then(|value| serde_json::from_value(value.clone()).ok()),
        suffix: run.suffix.clone(),
        validation_file: run.validation_file.clone(),
        integrations: run
            .integrations
            .as_ref()
            .map(|list| list.iter().cloned().map(Json::Text).collect()),
        seed: run.seed,
        custom_llm_provider: run.provider.clone(),
        extra_headers: run.extra_headers.clone(),
        extra_body: run
            .extra_body
            .as_ref()
            .and_then(|value| serde_json::from_value(value.clone()).ok()),
        params: CreateParams {
            api_base: Some(api_base.to_owned()),
            ..run.params.clone()
        },
        is_async: run.is_async,
    }
}
