use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::pillar::api_call::call_pillar_api;
use betterlite_guardrails::pillar::bool_config::{BoolSetting, resolve_bool_config};
use betterlite_guardrails::pillar::error::PillarError;
use betterlite_guardrails::pillar::error_fallback::handle_api_error;
use betterlite_guardrails::pillar::headers::{KeyContext, prepare_headers};
use betterlite_guardrails::pillar::model_provider::{CatalogResolver, ModelProviderResolver};
use betterlite_guardrails::pillar::payload::prepare_payload;
use betterlite_guardrails::pillar::response::process_pillar_response;
use betterlite_guardrails::pillar::settings::{FallbackAction, OnFlaggedAction, PillarSettings};
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use super::fail;
use betterlite_http::proxy;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_oracle_runner::case::UpstreamAnswer;
use betterlite_oracle_runner::mock::MockProvider;
use betterlite_oracle_runner::routing_mock_scripts::{MockScript, ProviderCall};
use std::slice;
use tokio::runtime::Builder;

const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Setting {
    AsyncMode,
    PersistSession,
    IncludeScanners,
    IncludeEvidence,
}

impl Setting {
    pub(super) const fn config(self) -> (&'static str, Option<bool>) {
        match self {
            Self::AsyncMode => ("PILLAR_ASYNC", None),
            Self::PersistSession => ("PILLAR_PERSIST", None),
            Self::IncludeScanners => ("PILLAR_INCLUDE_SCANNERS", Some(true)),
            Self::IncludeEvidence => ("PILLAR_INCLUDE_EVIDENCE", Some(true)),
        }
    }

    fn resolve(
        self,
        provided: Option<bool>,
        environment: &BTreeMap<String, String>,
    ) -> Option<bool> {
        let (env_var, default) = self.config();
        let setting = provided.map(BoolSetting::Boolean);
        resolve_bool_config(
            setting.as_ref(),
            Some(env_var),
            default,
            environment,
            env_var,
        )
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BoolCase {
    pub(super) setting: Setting,
    #[serde(default)]
    pub(super) provided: Option<bool>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProtectInput {
    settings: SettingsInput,
    #[serde(default)]
    key: KeyContext,
    data: Value,
    answer: Answer,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SettingsInput {
    api_key: Option<String>,
    on_flagged_action: Option<String>,
    async_mode: Option<bool>,
    persist_session: Option<bool>,
    include_scanners: Option<bool>,
    include_evidence: Option<bool>,
    fallback_on_error: Option<String>,
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    status: u16,
    body: Value,
}

fn pillar_settings(input: &SettingsInput) -> PillarSettings {
    let environment = &input.environment;
    PillarSettings {
        api_key: input.api_key.clone().map(SecretString::from),
        api_base: String::from("https://pillar.test"),
        on_flagged_action: input
            .on_flagged_action
            .as_deref()
            .and_then(OnFlaggedAction::parse)
            .unwrap_or(OnFlaggedAction::Monitor),
        async_mode: Setting::AsyncMode.resolve(input.async_mode, environment),
        persist_session: Setting::PersistSession.resolve(input.persist_session, environment),
        include_scanners: Setting::IncludeScanners
            .resolve(input.include_scanners, environment)
            .unwrap_or(true),
        include_evidence: Setting::IncludeEvidence
            .resolve(input.include_evidence, environment)
            .unwrap_or(true),
        fallback_on_error: input
            .fallback_on_error
            .as_deref()
            .and_then(FallbackAction::parse)
            .unwrap_or(FallbackAction::Allow),
        timeout: TIMEOUT,
    }
}

async fn call_and_process(
    client: &reqwest::Client,
    settings: &PillarSettings,
    key: &KeyContext,
    resolver: &(dyn ModelProviderResolver + Sync),
    data: &mut BTreeMap<String, GuardrailValue>,
) -> Option<u16> {
    let result = async {
        let headers = prepare_headers(settings, key)?;
        let payload = prepare_payload(data, resolver)?;
        let response = call_pillar_api(client, settings, &headers, &payload).await?;
        process_pillar_response(&response, data, settings)
    }
    .await;
    match result {
        Ok(()) => Some(200),
        Err(PillarError::Http(GuardrailFailure::HttpObject { status, .. })) => Some(status),
        Err(error) => match handle_api_error(&error, data.clone(), settings) {
            Ok(_) => Some(200),
            Err(PillarError::Http(GuardrailFailure::HttpObject { status, .. })) => Some(status),
            Err(_) => None,
        },
    }
}

fn captured_headers(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.to_lowercase(), value.clone()))
        .filter(|(name, _)| {
            name == "content-type" || name.starts_with("plr_") || name.starts_with("x-litellm-")
        })
        .collect()
}

fn captured_request(calls: &[ProviderCall]) -> Option<Value> {
    let call = calls.first()?;
    Some(json!({
        "method": call.request.method.to_uppercase(),
        "path": call.request.path.clone(),
        "headers": captured_headers(&call.request.headers),
        "body": call.request.body.clone(),
    }))
}

fn deliver(
    input: &ProtectInput,
    settings: &PillarSettings,
    data: &mut BTreeMap<String, GuardrailValue>,
    mock: MockProvider,
) -> Result<(Option<u16>, Vec<ProviderCall>), Box<HelperError>> {
    let client = proxy::base_builder()
        .build()
        .map_err(|error| fail("RequestError", error.to_string(), None))?;
    let catalog = BundledCatalog::default().get();
    let (known, custom, environment) = (BTreeSet::new(), BTreeSet::new(), BTreeMap::new());
    let registry = JSONProviderRegistry::default();
    let resolver = CatalogResolver {
        catalog: catalog.as_ref(),
        known_model_names: &known,
        custom_providers: &custom,
        environment: &environment,
        registry: &registry,
    };
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| fail("RuntimeError", error.to_string(), None))?;
    let outcome = runtime.block_on(call_and_process(
        &client, settings, &input.key, &resolver, data,
    ));
    let calls = mock
        .finish()
        .map_err(|error| fail("MockError", error.to_string(), None))?;
    Ok((outcome, calls))
}

pub(super) fn run(input: &ProtectInput) -> HelperResult {
    let mut settings = pillar_settings(&input.settings);
    let mut data = serde_json::from_value(input.data.clone())
        .map_err(|error| fail("ValueError", error.to_string(), None))?;
    let reply = UpstreamAnswer {
        status: input.answer.status,
        headers: BTreeMap::new(),
        body: Some(input.answer.body.clone()),
        events: None,
    };
    let script = MockScript::try_from(slice::from_ref(&reply))
        .map_err(|error| fail("MockError", error.to_string(), None))?;
    let mock =
        MockProvider::start(script).map_err(|error| fail("MockError", error.to_string(), None))?;
    settings.api_base.clone_from(&mock.base_url);
    let (outcome, calls) = deliver(input, &settings, &mut data, mock)?;
    Ok(json!({
        "request": captured_request(&calls),
        "data": serde_json::to_value(&data).map_err(|error| fail("ValueError", error.to_string(), None))?,
        "status": outcome,
    }))
}
