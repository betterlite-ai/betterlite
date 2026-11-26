mod scripted;

use std::sync::Arc;

use betterlite_auth::auth_model_lists::credential::{CredentialEntry, CredentialSnapshot};
use betterlite_config::tls::Verification;
use betterlite_core::api::realtime_connect::AZURE_DEFAULT_API_VERSION;
use betterlite_core::api::realtime_connection_plan::azure::{ad_token, azure_protocol};
use betterlite_core::api::realtime_connection_plan::metadata::RealtimeModelMetadata;
use betterlite_core::api::realtime_connection_plan::parameters::{
    model_params_with_stored_credentials, resolve_probe_parameters,
};
use betterlite_core::api::realtime_connection_plan::{
    AZURE_PROVIDER, OPENAI_DEFAULT_API_BASE, OPENAI_PROVIDER, REALTIME_PROTOCOL_KEY,
    RealtimeProbeDependencies, RealtimeProbeError, RealtimeProbeRequest, RealtimeProbeSettings,
    VERTEX_AI_PROVIDER, XAI_DEFAULT_API_BASE, XAI_PROVIDER, realtime_health_check,
};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder as RuntimeBuilder;
use tokio::runtime::Runtime;

use scripted::{RecordingDialer, ScriptedAzure, ScriptedVertex, ssl_kind};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = RuntimeBuilder::new_current_thread()
            .enable_all()
            .build()
            .map_err(failure)?;
        let mut answered = Vec::with_capacity(input.scenarios.len());
        for scenario in &input.scenarios {
            answered.push(answer(&runtime, scenario)?);
        }
        Ok(json!({ "scenarios": answered }))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    scenarios: Vec<Scenario>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    model: String,
    custom_llm_provider: String,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_version: Option<String>,
    #[serde(default)]
    realtime_protocol: Option<String>,
    #[serde(default)]
    model_params: JsonObject,
    #[serde(default)]
    stored: Vec<StoredCredential>,
    #[serde(default)]
    azure_ad_token: Option<String>,
    #[serde(default)]
    catalog: IndexMap<String, Json>,
    #[serde(default)]
    vertex_token: Option<String>,
    #[serde(default)]
    vertex_project: Option<String>,
    #[serde(default)]
    vertex_location: Option<String>,
    #[serde(default)]
    settings: InputSettings,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct StoredCredential {
    credential_name: String,
    credential_values: JsonObject,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct InputSettings {
    #[serde(default)]
    azure_realtime_protocol: Option<String>,
    #[serde(default = "default_timeout")]
    credential_timeout_seconds: f64,
    #[serde(default = "default_max_message_size")]
    max_message_size_bytes: usize,
}

fn default_timeout() -> f64 {
    20.0_f64
}

fn default_max_message_size() -> usize {
    16_777_216_usize
}

// the `or` default: an absent or empty value takes the default
fn configured_or(value: Option<String>, default: &str) -> String {
    value
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| String::from(default))
}

impl InputSettings {
    fn probe_settings(&self) -> RealtimeProbeSettings {
        RealtimeProbeSettings {
            azure_realtime_protocol: self.azure_realtime_protocol.clone(),
            credential_timeout_seconds: self.credential_timeout_seconds,
            max_message_size_bytes: self.max_message_size_bytes,
        }
    }
}

fn answer(runtime: &Runtime, scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let catalog_bytes = serde_json::to_vec(&scenario.catalog).map_err(failure)?;
    let catalog =
        Arc::new(ModelCatalog::parse(&catalog_bytes, UnknownFields::Warn).map_err(failure)?);
    let stored = CredentialSnapshot::new(
        scenario
            .stored
            .iter()
            .map(|entry| CredentialEntry {
                credential_name: entry.credential_name.clone(),
                credential_values: entry.credential_values.clone(),
            })
            .collect(),
    );
    let settings = scenario.settings.probe_settings();
    let request = RealtimeProbeRequest {
        model: scenario.model.clone(),
        custom_llm_provider: scenario.custom_llm_provider.clone(),
        api_key: scenario.api_key.as_deref().map(SecretString::from),
        api_base: scenario.api_base.clone(),
        api_version: scenario.api_version.clone(),
        realtime_protocol: scenario.realtime_protocol.clone(),
        model_params: scenario.model_params.clone(),
    };
    let merged = model_params_with_stored_credentials(&request.model_params, &stored);
    let resolved = resolve_probe_parameters(&request, &merged).map_err(failure)?;
    let azure = ScriptedAzure {
        token: scenario.azure_ad_token.as_deref().map(SecretString::from),
    };
    let vertex = ScriptedVertex::new(scenario, Arc::clone(&catalog));
    let dialer = RecordingDialer::default();
    let verification = Verification::Enabled;
    let metadata: &dyn RealtimeModelMetadata = catalog.as_ref();
    let dependencies = RealtimeProbeDependencies {
        stored: &stored,
        metadata,
        azure: &azure,
        vertex: &vertex,
        dialer: &dialer,
        verification: &verification,
    };
    let outcome = runtime.block_on(realtime_health_check(&request, &settings, &dependencies));
    let branch = match &outcome {
        Ok(()) => scenario.custom_llm_provider.as_str(),
        Err(RealtimeProbeError::UnsupportedModel { .. }) => "unsupported",
        Err(RealtimeProbeError::VertexSpeechModel { .. }) => "speech_to_text",
        Err(error) => return Err(failure(error)),
    };
    let connection = dialer.take();
    let protocol = if scenario.custom_llm_provider == AZURE_PROVIDER {
        Some(azure_protocol(
            request.realtime_protocol.as_deref(),
            merged.get(REALTIME_PROTOCOL_KEY),
            settings.azure_realtime_protocol.as_deref(),
        ))
    } else {
        None
    };
    let azure_token = if scenario.custom_llm_provider == AZURE_PROVIDER {
        ad_token(resolved.api_key.as_ref(), &merged, &azure)
            .map_err(failure)?
            .map(|token| token.expose_secret().to_owned())
    } else {
        None
    };
    let base = match (scenario.custom_llm_provider.as_str(), resolved.api_base) {
        (AZURE_PROVIDER, base) => json!(base.unwrap_or_default()),
        (OPENAI_PROVIDER, base) => {
            json!(configured_or(base, OPENAI_DEFAULT_API_BASE))
        }
        (XAI_PROVIDER, base) => json!(configured_or(base, XAI_DEFAULT_API_BASE)),
        (_, base) => json!(base),
    };
    let version = match scenario.custom_llm_provider.as_str() {
        AZURE_PROVIDER => json!(configured_or(
            resolved.api_version,
            AZURE_DEFAULT_API_VERSION
        )),
        _ => Value::Null,
    };
    let key = resolved
        .api_key
        .as_ref()
        .map(|key| key.expose_secret().to_owned());
    let observed = connection.map(|connection| {
        let headers: Vec<Value> = connection
            .headers
            .iter()
            .map(|(name, value)| json!([name, value]))
            .collect();
        (
            json!(connection.url),
            headers,
            json!(connection.max_size),
            json!(ssl_kind(&connection.ssl)),
        )
    });
    let (observed_url, headers, max_size, ssl) =
        observed.unwrap_or_else(|| (Value::Null, Vec::new(), Value::Null, Value::Null));
    let url = if branch == VERTEX_AI_PROVIDER {
        observed_url
    } else {
        Value::Null
    };
    Ok(json!({
        "branch": branch,
        "api_key": key,
        "api_base": base,
        "api_version": version,
        "protocol": protocol,
        "ad_token": azure_token,
        "url": url,
        "headers": headers,
        "max_size": max_size,
        "ssl": ssl,
    }))
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
