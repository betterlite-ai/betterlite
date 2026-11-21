use std::collections::BTreeMap;
use std::env;

use betterlite_azure::realtime::connect::RealtimeConnectSettings;
use betterlite_azure::realtime::http_config::RealtimeHttpSettings;
use betterlite_config::tls::Verification;
use betterlite_openai::realtime::url::QueryParams;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::azure_resource_dispatch::realtime::{self, Call, Outcome, SessionRoute};
use betterlite_provider::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::support::{OTHER, failure, runtime};

// B545's websocket plan and HTTP session views, each one read from the leaf's own settings
pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = runtime()?;
    let mut connects = Vec::with_capacity(input.queries.len());
    let mut http = Vec::with_capacity(input.http_setups.len());
    for step in &input.queries {
        connects.push(connect(&runtime, input, step)?);
    }
    for step in &input.http_setups {
        http.push(http_view(&runtime, step)?);
    }
    Ok(json!({"connects": connects, "http": http}))
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(super) model: String,
    #[serde(default)]
    pub(super) max_message_size_bytes: Option<u64>,
    #[serde(default)]
    pub(super) queries: Vec<QuerySetup>,
    #[serde(default)]
    pub(super) http_setups: Vec<HttpSetup>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct QuerySetup {
    pub(super) api_base: Option<String>,
    pub(super) api_version: Option<String>,
    pub(super) realtime_protocol: Option<String>,
    #[serde(default)]
    pub(super) query_params: Option<QueryParams>,
    pub(super) api_key: Option<String>,
    pub(super) azure_ad_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HttpSetup {
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) api_version: Option<String>,
    pub(super) litellm_api_base: Option<String>,
    pub(super) litellm_api_key: Option<String>,
    pub(super) ephemeral_key: String,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
}

fn connect(runtime: &Runtime, input: &Input, step: &QuerySetup) -> Result<Value, Box<HelperError>> {
    let key = step.api_key.clone().map(SecretString::from);
    let token = step.azure_ad_token.clone().map(SecretString::from);
    let verification = Verification::Enabled;
    let max_size = match input.max_message_size_bytes {
        Some(value) => usize::try_from(value).map_err(failure)?,
        None => 0_usize,
    };
    let settings = RealtimeConnectSettings {
        api_base: step.api_base.as_deref(),
        api_key: key.as_ref(),
        azure_ad_token: token.as_ref(),
        model: &input.model,
        api_version: step.api_version.as_deref(),
        realtime_protocol: step.realtime_protocol.as_deref(),
        query_params: step.query_params.as_ref(),
        ssl_verification: &verification,
        max_message_size_bytes: max_size,
    };
    let Outcome::Connection(plan) = through(runtime, Call::Connection(&settings))? else {
        return Err(failure(OTHER));
    };
    let headers: Vec<[String; 2]> = plan
        .headers
        .iter()
        .map(|(name, value)| [name.clone(), value.clone()])
        .collect();
    Ok(json!({
        "url": plan.url,
        "headers": headers,
        "max_size": plan.max_message_size_bytes,
        "ssl": plan.ssl,
    }))
}

fn http_view(runtime: &Runtime, step: &HttpSetup) -> Result<Value, Box<HelperError>> {
    let settings = RealtimeHttpSettings {
        litellm_api_base: step.litellm_api_base.clone(),
        litellm_api_key: step.litellm_api_key.clone().map(SecretString::from),
        azure_api_base: env::var("AZURE_API_BASE").ok(),
        azure_api_key: env::var("AZURE_API_KEY").ok().map(SecretString::from),
        azure_api_version: env::var("AZURE_API_VERSION").ok(),
    };
    let base = settings.api_base(step.api_base.as_deref());
    let key = settings.api_key(step.api_key.as_deref());
    let complete = url_outcome(
        runtime,
        Call::SessionUrl {
            route: SessionRoute::ClientSecrets,
            settings: &settings,
            base: Some(&base),
            version: step.api_version.as_deref(),
        },
    )?;
    let calls = url_outcome(
        runtime,
        Call::SessionUrl {
            route: SessionRoute::Calls,
            settings: &settings,
            base: Some(&base),
            version: step.api_version.as_deref(),
        },
    )?;
    let transcription = url_outcome(
        runtime,
        Call::SessionUrl {
            route: SessionRoute::Transcription,
            settings: &settings,
            base: Some(&base),
            version: step.api_version.as_deref(),
        },
    )?;
    let headers = headers_outcome(
        runtime,
        Call::SessionHeaders {
            headers: &step.headers,
            api_key: Some(&key),
        },
    )?;
    let calls_headers = headers_outcome(runtime, Call::CallsHeaders(&step.ephemeral_key))?;
    Ok(json!({
        "api_base": base,
        "api_key": key.expose_secret(),
        "complete_url": complete,
        "calls_url": calls,
        "transcription_url": transcription,
        "headers": headers,
        "calls_headers": calls_headers,
    }))
}

fn url_outcome(runtime: &Runtime, call: Call<'_>) -> Result<String, Box<HelperError>> {
    match through(runtime, call)? {
        Outcome::Url(url) => Ok(url),
        _ => Err(failure(OTHER)),
    }
}

fn headers_outcome(
    runtime: &Runtime,
    call: Call<'_>,
) -> Result<BTreeMap<String, String>, Box<HelperError>> {
    match through(runtime, call)? {
        Outcome::Headers(headers) => Ok(headers),
        _ => Err(failure(OTHER)),
    }
}

fn through(runtime: &Runtime, call: Call<'_>) -> Result<Outcome, Box<HelperError>> {
    let result = runtime.block_on(dispatch(AzureResourceOperation::Realtime(
        realtime::Dispatch { call },
    )));
    match result {
        Ok(AzureResourceResult::Realtime(outcome)) => Ok(outcome),
        Ok(_) => Err(failure(OTHER)),
        Err(AzureResourceError::Realtime(error)) => Err(failure(&error)),
        Err(error) => Err(failure(error)),
    }
}
