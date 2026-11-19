use std::collections::BTreeMap;
use std::future::Future;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::resource_sdk_settings::Settings;
use betterlite_core::api::fine_tuning_cancel_call::{
    FineTuningCancel, FineTuningCancelCall, FineTuningOptionalParams, OptionalTimeout,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use betterlite_openai::batches::sdk::TimeoutPhases;
use betterlite_openai::fine_tuning::job::FineTuneJob;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::Failure;
use super::result::kinds::SdkResult;
use dispatch::HttpDispatch;
use projection::failure;

mod dispatch;
mod projection;

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
