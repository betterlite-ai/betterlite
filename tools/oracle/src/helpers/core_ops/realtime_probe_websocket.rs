mod scripted;

use std::fmt::Display;
use std::sync::Arc;

use betterlite_auth::auth_model_lists::credential::{CredentialEntry, CredentialSnapshot};
use betterlite_config::tls::Verification;
use betterlite_core::api::realtime_connect::AZURE_DEFAULT_API_VERSION;
use betterlite_core::api::realtime_connection_plan::azure::azure_protocol;
use betterlite_core::api::realtime_connection_plan::metadata::RealtimeModelMetadata;
use betterlite_core::api::realtime_connection_plan::parameters::{
    ResolvedProbeParameters, merge_stored_credentials, resolve_probe_parameters,
};
use betterlite_core::api::realtime_connection_plan::{
    AZURE_PROVIDER, OPENAI_DEFAULT_API_BASE, OPENAI_PROVIDER, ProbeConnection,
    REALTIME_PROTOCOL_KEY, RealtimeProbeDependencies, RealtimeProbeError, RealtimeProbeRequest,
    RealtimeProbeSettings, VERTEX_AI_PROVIDER, XAI_DEFAULT_API_BASE, XAI_PROVIDER,
    realtime_health_check,
};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use secrecy::SecretString;
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
            .map_err(|error| failure(&error))?;
        let mut answered = Vec::with_capacity(input.scenarios.len());
        for scenario in &input.scenarios {
            let probe = Probe::new(scenario)?;
            let (branch, connection) = probe.health(&runtime)?;
            answered.push(probe.answer(&branch, connection.as_ref()));
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

const fn default_timeout() -> f64 {
    20.0_f64
}

const fn default_max_message_size() -> usize {
    0x0100_0000_usize
}

// the `or` default: an absent or empty value takes the default
fn configured_or(value: Option<String>, default: &str) -> String {
    value
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| String::from(default))
}

fn stored_credentials(scenario: &Scenario) -> CredentialSnapshot {
    CredentialSnapshot::new(
        scenario
            .stored
            .iter()
            .map(|entry| CredentialEntry {
                credential_name: entry.credential_name.clone(),
                credential_values: entry.credential_values.clone(),
            })
            .collect(),
    )
}

// the `or` default each provider's url builder applies to a configured api_base
fn base_value(provider: &str, configured: Option<String>) -> Value {
    match (provider, configured) {
        (AZURE_PROVIDER, base) => json!(base.unwrap_or_default()),
        (OPENAI_PROVIDER, base) => json!(configured_or(base, OPENAI_DEFAULT_API_BASE)),
        (XAI_PROVIDER, base) => json!(configured_or(base, XAI_DEFAULT_API_BASE)),
        (_, base) => json!(base),
    }
}

// one scenario's resolved inputs, built once so each step takes only the fields it reads
struct Probe<'case> {
    scenario: &'case Scenario,
    catalog: Arc<ModelCatalog>,
    stored: CredentialSnapshot,
    settings: RealtimeProbeSettings,
    request: RealtimeProbeRequest,
    merged: JsonObject,
    resolved: ResolvedProbeParameters,
    azure: ScriptedAzure,
}

impl<'case> Probe<'case> {
    fn new(scenario: &'case Scenario) -> Result<Self, Box<HelperError>> {
        let catalog_bytes =
            serde_json::to_vec(&scenario.catalog).map_err(|error| failure(&error))?;
        let catalog = Arc::new(
            ModelCatalog::parse(&catalog_bytes, UnknownFields::Warn)
                .map_err(|error| failure(&error))?,
        );
        let stored = stored_credentials(scenario);
        let settings = RealtimeProbeSettings {
            azure_realtime_protocol: scenario.settings.azure_realtime_protocol.clone(),
            credential_timeout_seconds: scenario.settings.credential_timeout_seconds,
            max_message_size_bytes: scenario.settings.max_message_size_bytes,
        };
        let request = RealtimeProbeRequest {
            model: scenario.model.clone(),
            custom_llm_provider: scenario.custom_llm_provider.clone(),
            api_key: scenario.api_key.as_deref().map(SecretString::from),
            api_base: scenario.api_base.clone(),
            api_version: scenario.api_version.clone(),
            realtime_protocol: scenario.realtime_protocol.clone(),
            model_params: scenario.model_params.clone(),
        };
        let merged = merge_stored_credentials(&request.model_params, &stored);
        let resolved =
            resolve_probe_parameters(&request, &merged).map_err(|error| failure(&error))?;
        let azure = ScriptedAzure {
            token: scenario.azure_ad_token.as_deref().map(SecretString::from),
        };
        Ok(Self {
            scenario,
            catalog,
            stored,
            settings,
            request,
            merged,
            resolved,
            azure,
        })
    }

    // the probe opens exactly one connection and answers its branch; the recording is the handshake it asked for
    fn health(
        &self,
        runtime: &Runtime,
    ) -> Result<(String, Option<ProbeConnection>), Box<HelperError>> {
        let vertex = ScriptedVertex::new(self.scenario, Arc::clone(&self.catalog));
        let dialer = RecordingDialer::default();
        let verification = Verification::Enabled;
        let metadata: &dyn RealtimeModelMetadata = self.catalog.as_ref();
        let dependencies = RealtimeProbeDependencies {
            stored: &self.stored,
            metadata,
            azure: &self.azure,
            vertex: &vertex,
            dialer: &dialer,
            verification: &verification,
        };
        let outcome = runtime.block_on(realtime_health_check(
            &self.request,
            &self.settings,
            &dependencies,
        ));
        let branch = match outcome {
            Ok(()) => self.scenario.custom_llm_provider.clone(),
            Err(RealtimeProbeError::UnsupportedModel { .. }) => String::from("unsupported"),
            Err(RealtimeProbeError::VertexSpeechModel { .. }) => String::from("speech_to_text"),
            Err(error) => return Err(failure(&error)),
        };
        Ok((branch, dialer.take()))
    }

    // the resolved plan plus the one handshake; the Azure fields answer only for an Azure scenario
    fn answer(&self, branch: &str, connection: Option<&ProbeConnection>) -> Value {
        let provider = self.scenario.custom_llm_provider.as_str();
        let is_azure = provider == AZURE_PROVIDER;
        let protocol = is_azure.then(|| {
            azure_protocol(
                self.request.realtime_protocol.as_deref(),
                self.merged.get(REALTIME_PROTOCOL_KEY),
                self.settings.azure_realtime_protocol.as_deref(),
            )
        });
        let api_version = is_azure
            .then(|| configured_or(self.resolved.api_version.clone(), AZURE_DEFAULT_API_VERSION));
        let url = connection
            .as_ref()
            .filter(|_| branch == VERTEX_AI_PROVIDER)
            .map(|handshake| handshake.url.clone());
        let headers: Vec<Value> = connection
            .as_ref()
            .map(handshake_headers)
            .unwrap_or_default();
        let max_size = connection.as_ref().map(|handshake| handshake.max_size);
        let ssl = connection
            .as_ref()
            .map(|handshake| ssl_kind(&handshake.ssl));
        json!({
            "branch": branch,
            "api_base": base_value(provider, self.resolved.api_base.clone()),
            "api_version": api_version,
            "protocol": protocol,
            "url": url,
            "headers": headers,
            "max_size": max_size,
            "ssl": ssl,
        })
    }
}

// websockets.connect's headers as the comparison reads them; an absent handshake leaves an empty list
fn handshake_headers(handshake: &ProbeConnection) -> Vec<Value> {
    handshake
        .headers
        .iter()
        .map(|(name, value)| json!([name, value]))
        .collect()
}

fn failure<Error>(error: &Error) -> Box<HelperError>
where
    Error: Display,
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
