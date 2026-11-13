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
use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::{MockScript, Outcome, ScriptSpec, ScriptedReply, Wait};

const SCRIPT_DEADLINE_MILLIS: u64 = 30_000;
const TIMEOUT: Duration = Duration::from_secs(5);

const CONTEXT_HEADERS: [&str; 7] = [
    "x-litellm-key-name",
    "x-litellm-key-alias",
    "x-litellm-user-id",
    "x-litellm-user-email",
    "x-litellm-team-id",
    "x-litellm-team-name",
    "x-litellm-org-id",
];

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Setting {
    AsyncMode,
    PersistSession,
    IncludeScanners,
    IncludeEvidence,
}

impl Setting {
    pub(super) const fn env_var(self) -> &'static str {
        match self {
            Self::AsyncMode => "PILLAR_ASYNC",
            Self::PersistSession => "PILLAR_PERSIST",
            Self::IncludeScanners => "PILLAR_INCLUDE_SCANNERS",
            Self::IncludeEvidence => "PILLAR_INCLUDE_EVIDENCE",
        }
    }

    pub(super) const fn default_value(self) -> Option<bool> {
        match self {
            Self::AsyncMode | Self::PersistSession => None,
            Self::IncludeScanners | Self::IncludeEvidence => Some(true),
        }
    }

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::AsyncMode => "async_mode",
            Self::PersistSession => "persist_session",
            Self::IncludeScanners => "include_scanners",
            Self::IncludeEvidence => "include_evidence",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BoolCase {
    pub(super) setting: Setting,
    #[serde(default)]
    pub(super) provided: Option<bool>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProtectInput {
    settings: SettingsInput,
    #[serde(default)]
    key: KeyInput,
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

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct KeyInput {
    key_name: Option<String>,
    key_alias: Option<String>,
    user_id: Option<String>,
    user_email: Option<String>,
    team_id: Option<String>,
    team_alias: Option<String>,
    org_id: Option<String>,
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
        async_mode: resolve_bool_config(
            input.async_mode.map(BoolSetting::Boolean).as_ref(),
            Some("PILLAR_ASYNC"),
            None,
            environment,
            "async_mode",
        ),
        persist_session: resolve_bool_config(
            input.persist_session.map(BoolSetting::Boolean).as_ref(),
            Some("PILLAR_PERSIST"),
            None,
            environment,
            "persist_session",
        ),
        include_scanners: resolve_bool_config(
            input.include_scanners.map(BoolSetting::Boolean).as_ref(),
            Some("PILLAR_INCLUDE_SCANNERS"),
            Some(true),
            environment,
            "include_scanners",
        )
        .unwrap_or(true),
        include_evidence: resolve_bool_config(
            input.include_evidence.map(BoolSetting::Boolean).as_ref(),
            Some("PILLAR_INCLUDE_EVIDENCE"),
            Some(true),
            environment,
            "include_evidence",
        )
        .unwrap_or(true),
        fallback_on_error: input
            .fallback_on_error
            .as_deref()
            .and_then(FallbackAction::parse)
            .unwrap_or(FallbackAction::Allow),
        timeout: TIMEOUT,
    }
}

fn key_context(key: &KeyInput) -> KeyContext {
    KeyContext {
        key_name: key.key_name.clone(),
        key_alias: key.key_alias.clone(),
        user_id: key.user_id.clone(),
        user_email: key.user_email.clone(),
        team_id: key.team_id.clone(),
        team_alias: key.team_alias.clone(),
        org_id: key.org_id.clone(),
    }
}

fn guardrail_data(value: &Value) -> Result<BTreeMap<String, GuardrailValue>, Box<HelperError>> {
    serde_json::from_value(value.clone())
        .map_err(|error| fail("ValueError", error.to_string(), None))
}

fn start_mock(answer: &Answer) -> Result<MockProvider, Box<HelperError>> {
    let reply = ScriptedReply {
        wait: Wait::None,
        then: Outcome::Answer(UpstreamAnswer {
            status: answer.status,
            headers: BTreeMap::new(),
            body: Some(answer.body.clone()),
            events: None,
        }),
    };
    let script = MockScript::try_from(ScriptSpec {
        deadline_ms: SCRIPT_DEADLINE_MILLIS,
        replies: vec![reply],
    })
    .map_err(|error| fail("MockError", error.to_string(), None))?;
    MockProvider::start(script).map_err(|error| fail("MockError", error.to_string(), None))
}

async fn call_and_process(
    client: &reqwest::Client,
    settings: &PillarSettings,
    key: &KeyContext,
    resolver: &dyn ModelProviderResolver,
    data: &mut BTreeMap<String, GuardrailValue>,
) -> Option<u16> {
    let prepared = prepare_headers(settings, key)
        .and_then(|headers| prepare_payload(data, resolver).map(|payload| (headers, payload)));
    let result = match prepared {
        Ok((headers, payload)) => call_pillar_api(client, settings, &headers, &payload)
            .await
            .and_then(|response| process_pillar_response(&response, data, settings)),
        Err(error) => Err(error),
    };
    match result {
        Ok(()) => Some(200),
        Err(error) => {
            if let Some(status) = http_status(&error) {
                return Some(status);
            }
            match handle_api_error(&error, data.clone(), settings) {
                Ok(_) => Some(200),
                Err(failure) => http_status(&failure),
            }
        }
    }
}

fn http_status(error: &PillarError) -> Option<u16> {
    match error {
        PillarError::Http(GuardrailFailure::HttpObject { status, .. }) => Some(*status),
        PillarError::MissingApiKey
        | PillarError::Request { .. }
        | PillarError::Status { .. }
        | PillarError::Response { .. }
        | PillarError::ResponseTooLarge { .. }
        | PillarError::InvalidHeaderName { .. }
        | PillarError::InvalidHeaderValue { .. }
        | PillarError::Field { .. }
        | PillarError::BoolValue { .. }
        | PillarError::Http(_) => None,
    }
}

fn captured_headers(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            let lowered = name.to_lowercase();
            let kept = lowered == "content-type"
                || lowered.starts_with("plr_")
                || CONTEXT_HEADERS.contains(&lowered.as_str());
            kept.then_some((lowered, value.clone()))
        })
        .collect()
}

pub(super) fn run(input: &ProtectInput) -> HelperResult {
    let mut settings = pillar_settings(&input.settings);
    let key = key_context(&input.key);
    let mut data = guardrail_data(&input.data)?;
    let mock = start_mock(&input.answer)?;
    settings.api_base = mock.base_url.clone();
    let client = betterlite_http::proxy::base_builder()
        .build()
        .map_err(|error| fail("RequestError", error.to_string(), None))?;
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let known = BTreeSet::new();
    let custom = BTreeSet::new();
    let environment = BTreeMap::new();
    let registry = JSONProviderRegistry::default();
    let resolver = CatalogResolver {
        catalog: catalog.as_ref(),
        known_model_names: &known,
        custom_providers: &custom,
        environment: &environment,
        registry: &registry,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| fail("RuntimeError", error.to_string(), None))?;
    let outcome = runtime.block_on(call_and_process(
        &client, &settings, &key, &resolver, &mut data,
    ));
    let calls = mock
        .finish()
        .map_err(|error| fail("MockError", error.to_string(), None))?;
    let request = calls.first().map(|call| {
        json!({
            "method": call.request.method.to_uppercase(),
            "path": call.request.path.clone(),
            "headers": captured_headers(&call.request.headers),
            "body": call.request.body.clone(),
        })
    });
    Ok(json!({
        "request": request,
        "data": serde_json::to_value(&data).map_err(|error| fail("ValueError", error.to_string(), None))?,
        "status": outcome,
    }))
}
