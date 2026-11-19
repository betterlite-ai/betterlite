use super::call::Run;
use super::client::{self, HttpBuilder};
use super::outcome::{Outcome, RunOutcome, job_view};
use super::{azure, vertex};
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::{Failure, kinds::SdkResult};
use betterlite_azure::fine_tuning::error::AzureFineTuneError;
use betterlite_core::api::fine_tuning_create::connection::CreateParams;
use betterlite_core::api::fine_tuning_create::service::FineTuningCreate;
use betterlite_core::api::fine_tuning_create::{
    CreateFineTuningJobCall, FineTuningCreateDispatch, FineTuningCreateError,
    FineTuningCreateTarget, OpenAiCreateCall,
};
use betterlite_core::api::resource_sdk_settings::ResourceSdkSettings;
use betterlite_core::api::resource_sdk_settings::timeout::FineTuningTimeout;
use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
use betterlite_models::decisions::Json;
use betterlite_openai::fine_tuning::client::{ClientSettings, Connection};
use betterlite_openai::fine_tuning::error::FineTuneError;
use betterlite_openai::fine_tuning::job::FineTuneJob;
use betterlite_openai::fine_tuning::{JobRequest, request_fine_tuning_job};
use betterlite_vertex::vertex_fine_tuning::execution::TuningExecutionError;
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use thiserror::Error;
use tokio::task::{JoinError, JoinSet};

#[derive(Debug, Error)]
pub(crate) enum SendError {
    #[error(transparent)]
    OpenAi(#[from] FineTuneError),
    #[error(transparent)]
    Azure(#[from] AzureFineTuneError),
    #[error(transparent)]
    Vertex(#[from] TuningExecutionError),
    #[error("cannot resolve the oracle proxy policy: {0}")]
    ProxyPolicy(#[from] EnvError),
    #[error("cannot build the oracle HTTP client: {0}")]
    Client(#[source] Box<HttpError>),
    #[error("the oracle base URL is not valid: {0}")]
    BaseUrl(#[from] url::ParseError),
    #[error("the oracle blocking create task stopped: {0}")]
    Join(#[from] JoinError),
    #[error("the oracle blocking create task set is empty")]
    Empty,
}
pub(crate) async fn run(
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
        FineTuningCreateError::Dispatch(
            SendError::OpenAi(FineTuneError::Sdk { source })
            | SendError::Azure(AzureFineTuneError::Sdk { source }),
        ) => Failure {
            class: client::error_class(source.status).to_owned(),
            status: source.status,
            kind: Some(source.error_type.clone()),
            param: Some(source.param.clone()),
            code: Some(source.code.clone()),
            message: source.message.clone(),
        },
        FineTuningCreateError::Timeout(_)
        | FineTuningCreateError::Data(_)
        | FineTuningCreateError::Dispatch(_) => exception_failure(&error.to_string()),
    }
}
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
            let mut tasks = JoinSet::new();
            match target {
                FineTuningCreateTarget::OpenAi(call) => {
                    tasks.spawn_blocking(move || openai(call));
                }
                FineTuningCreateTarget::Azure(call) => {
                    tasks.spawn_blocking(move || azure::create(call));
                }
                FineTuningCreateTarget::Vertex(call) => return vertex::create(call, urls).await,
            }
            tasks.join_next().await.transpose()?.ok_or(SendError::Empty)
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
pub(crate) fn seconds(timeout: FineTuningTimeout) -> Option<f64> {
    match timeout {
        FineTuningTimeout::Seconds(value) => Some(value),
        FineTuningTimeout::Structured(found) => {
            found.read.or(found.connect).or(found.write).or(found.pool)
        }
    }
}
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
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or_default(),
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
