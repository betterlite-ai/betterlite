use std::collections::BTreeMap;
use std::future::Future;
use std::string::FromUtf8Error;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::resource_sdk_settings::Settings;
use betterlite_core::api::fine_tuning_cancel_call::{
    AzureFineTuningCancel, FineTuningCancel, FineTuningCancelCall, FineTuningCancelDispatch,
    FineTuningCancelError, FineTuningCancelTarget, FineTuningOptionalParams,
    OpenAiFineTuningCancel, OptionalTimeout,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::batches::sdk::TimeoutPhases;
use betterlite_openai::fine_tuning::error::FineTuneError;
use betterlite_openai::fine_tuning::job::{FineTuneJob, decode_fine_tune_job};
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue, Method};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;
use super::result::{Failure, Nullable};

const DEADLINE: Duration = Duration::from_secs(30);
const JSON_CONTENT: &str = "application/json";
// the openai client sends the organization it was built with as a header
const ORGANIZATION: &str = "openai-organization";

fn openai_provider() -> Option<String> {
    Some("openai".to_owned())
}

// one cancel call; runs execute in order against the same settings the case env loads
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub(crate) runs: Vec<Run>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub(crate) fine_tuning_job_id: String,
    #[serde(default = "openai_provider")]
    pub(crate) custom_llm_provider: Option<String>,
    // who supplies the api base: the call's params or the provider's environment variable
    #[serde(default)]
    pub(crate) base: Base,
    #[serde(default)]
    pub(crate) asynchronous: bool,
    #[serde(default)]
    pub(crate) extra_headers: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub(crate) extra_body: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub(crate) params: Params,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Base {
    #[default]
    Param,
    Env,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    pub(crate) api_key: Option<String>,
    pub(crate) api_version: Option<String>,
    pub(crate) organization: Option<String>,
    pub(crate) timeout: Option<Timeout>,
    pub(crate) request_timeout: Option<f64>,
    pub(crate) max_retries: Option<i64>,
    pub(crate) azure_ad_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Timeout {
    Seconds(f64),
    Text(String),
    Phases(Phases),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Phases {
    connect: Option<f64>,
    read: Option<f64>,
    write: Option<f64>,
    pool: Option<f64>,
}

impl From<&Params> for FineTuningOptionalParams {
    fn from(params: &Params) -> Self {
        Self {
            api_base: None,
            api_key: params
                .api_key
                .as_ref()
                .map(|key| SecretString::from(key.clone())),
            api_version: params.api_version.clone(),
            organization: params.organization.clone(),
            timeout: params.timeout.as_ref().map(optional_timeout),
            request_timeout: params.request_timeout,
            max_retries: params.max_retries,
            azure_ad_token: params
                .azure_ad_token
                .as_ref()
                .map(|token| SecretString::from(token.clone())),
        }
    }
}

fn optional_timeout(timeout: &Timeout) -> OptionalTimeout {
    match timeout {
        Timeout::Seconds(seconds) => OptionalTimeout::Seconds(*seconds),
        Timeout::Text(text) => OptionalTimeout::Text(text.clone()),
        Timeout::Phases(phases) => OptionalTimeout::Phases(TimeoutPhases {
            connect: phases.connect,
            read: phases.read,
            write: phases.write,
            pool: phases.pool,
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub(crate) runs: Vec<RunOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOutcome {
    pub(crate) job: Option<Json>,
    pub(crate) error: Option<Failure>,
}

pub(crate) struct HttpDispatch {
    client: ProxiedClient,
}

// the failure a dispatch reports as the call's own, before the outcome names its class
#[derive(Debug, Error)]
pub enum CancelFailure {
    #[error("the api key is not set")]
    MissingKey,
    #[error("the api base is not set")]
    MissingBase,
    #[error("the cancel call needs a credential a request header cannot carry")]
    InvalidHeader,
    #[error("the provider answered with status {status}")]
    Status { status: u16, body: String },
    #[error("the provider answered with status {status} and a body that is not UTF-8")]
    StatusUtf8 {
        status: u16,
        #[source]
        source: FromUtf8Error,
    },
    #[error(transparent)]
    Url(#[from] url::ParseError),
    #[error(transparent)]
    Transport(#[from] Box<TransportError>),
    #[error(transparent)]
    Decode(#[from] FineTuneError),
}

impl FineTuningCancelDispatch for HttpDispatch {
    type Error = CancelFailure;

    async fn cancel(
        &self,
        target: FineTuningCancelTarget<'_>,
    ) -> Result<FineTuneJob, CancelFailure> {
        let (request, is_azure) = match &target {
            FineTuningCancelTarget::OpenAi(openai) => (openai_request(openai)?, false),
            FineTuningCancelTarget::Azure(azure) => (azure_request(azure)?, true),
        };
        let response = self
            .client
            .send_buffered(request)
            .await
            .map_err(|source| CancelFailure::Transport(Box::new(source)))?;
        if !(200..300).contains(&response.status) {
            let body =
                String::from_utf8(response.body).map_err(|source| CancelFailure::StatusUtf8 {
                    status: response.status,
                    source,
                })?;
            return Err(CancelFailure::Status {
                status: response.status,
                body,
            });
        }
        decode_fine_tune_job(&response.body, is_azure).map_err(CancelFailure::Decode)
    }
}

impl CallKind for Call {
    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        send(self, api_base, env)
    }
}

async fn send(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let client = super::vector_query_embedding::outbound_client(env)?;
    let mut runs = Vec::with_capacity(call.runs.len());
    for run in &call.runs {
        runs.push(run_once(run, api_base, env, &client).await?);
    }
    Ok(SdkResult::FineTuningCancel {
        value: Box::new(Outcome { runs }),
    })
}

async fn run_once(
    run: &Run,
    api_base: &str,
    env: &BTreeMap<String, String>,
    client: &ProxiedClient,
) -> Result<RunOutcome, SdkHttpError> {
    let mut run_env = env.clone();
    let params = prepared_params(run, api_base, &mut run_env);
    let service = FineTuningCancel {
        dispatch: HttpDispatch {
            client: client.clone(),
        },
        settings: environment_settings(&run_env)?,
    };
    let request = FineTuningCancelCall {
        fine_tuning_job_id: run.fine_tuning_job_id.clone(),
        custom_llm_provider: run.custom_llm_provider.clone(),
        extra_headers: run.extra_headers.clone(),
        extra_body: run.extra_body.clone(),
        params,
        acancel_fine_tuning_job: run.asynchronous.then_some(Json::Bool(true)),
        client: None,
    };
    match service.run(&request).await {
        Ok(job) => Ok(RunOutcome {
            job: Some(job_json(&job)?),
            error: None,
        }),
        Err(error) => Ok(RunOutcome {
            job: None,
            error: Some(failure(&error)),
        }),
    }
}

// the harness owns the mock, so the mode a run picks supplies that base and leaves the other one a decoy
fn prepared_params(
    run: &Run,
    api_base: &str,
    env: &mut BTreeMap<String, String>,
) -> FineTuningOptionalParams {
    let mut params = FineTuningOptionalParams::from(&run.params);
    let variable = match run.custom_llm_provider.as_deref() {
        Some("azure") => "AZURE_API_BASE",
        _ => "OPENAI_BASE_URL",
    };
    let decoy = format!("{api_base}/env-base");
    let from_environment = if run.base == Base::Env {
        api_base
    } else {
        decoy.as_str()
    };
    env.insert(variable.to_owned(), from_environment.to_owned());
    if run.base == Base::Param {
        params.api_base = Some(api_base.to_owned());
    }
    params
}

fn environment_settings(env: &BTreeMap<String, String>) -> Result<Settings, SdkHttpError> {
    let config = ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    Ok(config.sections.resource_sdk_settings.clone())
}

fn openai_request(target: &OpenAiFineTuningCancel<'_>) -> Result<OutboundRequest, CancelFailure> {
    let Some(api_key) = target.api_key else {
        return Err(CancelFailure::MissingKey);
    };
    let mut headers = json_headers();
    let authorization = format!("Bearer {}", api_key.expose_secret());
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&authorization).map_err(|_source| CancelFailure::InvalidHeader)?,
    );
    if let Some(organization) = target.organization {
        headers.insert(
            ORGANIZATION,
            HeaderValue::from_str(organization).map_err(|_source| CancelFailure::InvalidHeader)?,
        );
    }
    let url = format!(
        "{}/fine_tuning/jobs/{}/cancel",
        target.api_base.trim_end_matches('/'),
        target.values.job_id
    );
    Ok(OutboundRequest {
        method: Method::POST,
        url: Url::parse(&url)?,
        query: Vec::new(),
        headers,
        body: RequestBody::Empty,
        deadline: DEADLINE,
    })
}

fn azure_request(target: &AzureFineTuningCancel<'_>) -> Result<OutboundRequest, CancelFailure> {
    let Some(api_base) = target.api_base else {
        return Err(CancelFailure::MissingBase);
    };
    let Some(api_key) = target.api_key else {
        return Err(CancelFailure::MissingKey);
    };
    let mut headers = json_headers();
    headers.insert(
        "api-key",
        HeaderValue::from_str(api_key.expose_secret())
            .map_err(|_source| CancelFailure::InvalidHeader)?,
    );
    let url = format!(
        "{}/openai/fine_tuning/jobs/{}/cancel",
        api_base.trim_end_matches('/'),
        target.values.job_id
    );
    let query = target
        .api_version
        .map(|version| vec![("api-version".to_owned(), version.to_owned())])
        .unwrap_or_default();
    Ok(OutboundRequest {
        method: Method::POST,
        url: Url::parse(&url)?,
        query,
        headers,
        body: RequestBody::Empty,
        deadline: DEADLINE,
    })
}

fn json_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT));
    headers.insert(ACCEPT, HeaderValue::from_static(JSON_CONTENT));
    headers
}

// the class each failure projects to; only the SDK's status errors carry a status, type, param and code
fn failure(error: &FineTuningCancelError<CancelFailure>) -> Failure {
    match error {
        FineTuningCancelError::Unsupported { .. } => {
            null_failure("BadRequestError", 400, error.to_string())
        }
        FineTuningCancelError::Timeout(_)
        | FineTuningCancelError::Dispatch(
            CancelFailure::MissingBase | CancelFailure::InvalidHeader,
        ) => null_failure("ValueError", 0, error.to_string()),
        FineTuningCancelError::Dispatch(CancelFailure::MissingKey) => {
            null_failure("OpenAIError", 0, error.to_string())
        }
        FineTuningCancelError::Dispatch(CancelFailure::Status { status, body }) => {
            status_failure(*status, body)
        }
        FineTuningCancelError::Dispatch(CancelFailure::StatusUtf8 { status, .. }) => {
            null_failure(status_class(*status), *status, error.to_string())
        }
        FineTuningCancelError::Dispatch(
            CancelFailure::Url(_) | CancelFailure::Transport(_) | CancelFailure::Decode(_),
        ) => null_failure("APIConnectionError", 0, error.to_string()),
    }
}

// an error the SDK raises before or without a provider body leaves type, param and code null
fn null_failure(class: &str, status: u16, message: String) -> Failure {
    Failure {
        class: class.to_owned(),
        status,
        kind: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    }
}

// a status error reads type, param and code out of the body and renders the status with the raw body
fn status_failure(status: u16, body: &str) -> Failure {
    let (kind, param, code) = body_fields(body);
    Failure {
        class: status_class(status).to_owned(),
        status,
        kind,
        param,
        code,
        message: format!("Error code: {status} - {body}"),
    }
}

const fn status_class(status: u16) -> &'static str {
    match status {
        400 => "BadRequestError",
        401 => "AuthenticationError",
        403 => "PermissionDeniedError",
        404 => "NotFoundError",
        409 => "ConflictError",
        422 => "UnprocessableEntityError",
        429 => "RateLimitError",
        _ if status >= 500 => "InternalServerError",
        _ => "APIStatusError",
    }
}

// the SDK reads type, param and code from the body, or from its "error" member when it has one
fn body_fields(body: &str) -> (Nullable, Nullable, Nullable) {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    let candidate = match &parsed {
        serde_json::Value::Object(fields) => fields.get("error").unwrap_or(&parsed),
        _ => &parsed,
    };
    let serde_json::Value::Object(fields) = candidate else {
        return (Some(None), Some(None), Some(None));
    };
    (
        nullable(fields.get("type")),
        nullable(fields.get("param")),
        nullable(fields.get("code")),
    )
}

fn nullable(value: Option<&serde_json::Value>) -> Nullable {
    match value {
        None | Some(serde_json::Value::Null) => Some(None),
        Some(serde_json::Value::String(text)) => Some(Some(text.clone())),
        Some(other) => Some(Some(other.to_string())),
    }
}

fn job_json(job: &FineTuneJob) -> Result<Json, SdkHttpError> {
    let value = serde_json::to_value(job).map_err(SdkHttpError::Serialize)?;
    Ok(decision_json(value))
}

// the workspace serializes numbers with arbitrary precision, so the two JSON types convert directly
fn decision_json(value: serde_json::Value) -> Json {
    match value {
        serde_json::Value::Null => Json::Null,
        serde_json::Value::Bool(flag) => Json::Bool(flag),
        serde_json::Value::Number(number) => Json::Number(number),
        serde_json::Value::String(text) => Json::Text(text),
        serde_json::Value::Array(items) => {
            Json::List(items.into_iter().map(decision_json).collect())
        }
        serde_json::Value::Object(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key, decision_json(value)))
                .collect(),
        ),
    }
}
