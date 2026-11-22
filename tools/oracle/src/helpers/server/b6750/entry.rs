use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::resolution::{
    ProviderIdentity, ResolutionError, ResolutionRequest, ResolvedModel,
};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use betterlite_redact::secret::Redactor;
use betterlite_server::health_check_entry::ports::{CatalogMode, CatalogModes, ProviderResolver};
use betterlite_server::health_check_entry::{
    HealthCheckEntry, HealthCheckFailure, HealthCheckOutcome,
};
use betterlite_server::health_probe_modes::logging::HealthProbeLogging;
use betterlite_server::health_probe_modes::params::HealthProbeParams;
use chrono::DateTime;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

use super::calls::{HealthCheckDocuments, ProbeRecorder, ScriptedCheapest};
use super::{Resolution, Step};

const NOW: &str = "2026-01-02T03:04:05+00:00";
const FIXED_ID: &str = "11111111-1111-4111-8111-111111111111";
const STACK_SEPARATOR: &str = "\nstack trace: ";
const MISSING_MODE_SEPARATOR: &str = "\nstacktrace: ";

// the resolver call the case observes: what the entry asked, and the answer the case scripted
pub(super) struct ScriptedResolver {
    pub(super) answer: Option<Resolution>,
    pub(super) requested: Mutex<Option<Requested>>,
}

pub(super) struct Requested {
    pub(super) model: String,
    pub(super) custom_llm_provider: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
}

impl ScriptedResolver {
    pub(super) fn new(answer: Option<Resolution>) -> Self {
        Self {
            answer,
            requested: Mutex::new(None),
        }
    }
}

impl ProviderResolver for ScriptedResolver {
    fn resolve(&self, request: &ResolutionRequest<'_>) -> Result<ResolvedModel, ResolutionError> {
        let requested = Requested {
            model: request.model.to_owned(),
            custom_llm_provider: request.custom_llm_provider.map(str::to_owned),
            api_base: request.api_base.map(str::to_owned),
            api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
        };
        *self
            .requested
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(requested);
        let Some(answer) = &self.answer else {
            return Err(ResolutionError::MissingProvider {
                model: request.model.to_owned(),
            });
        };
        Ok(ResolvedModel {
            model: answer.model.clone(),
            provider: ProviderIdentity::Custom(answer.provider.clone()),
            api_base: None,
            api_key: None,
        })
    }
}

pub(super) struct ScriptedCatalog {
    modes: BTreeMap<String, Option<String>>,
}

impl ScriptedCatalog {
    pub(super) fn new(modes: &BTreeMap<String, Option<String>>) -> Self {
        Self {
            modes: modes.clone(),
        }
    }
}

impl CatalogModes for ScriptedCatalog {
    fn catalog_mode(&self, model: &str) -> CatalogMode {
        match self.modes.get(model) {
            None => CatalogMode::NotListed,
            Some(None) => CatalogMode::NoMode,
            Some(Some(mode)) => CatalogMode::Mode(mode.clone()),
        }
    }
}

pub(super) async fn run_step(step: &Step) -> Result<Value, Box<HelperError>> {
    let entries: IndexMap<String, Value> = step.params.clone().into_iter().collect();
    let params = HealthProbeParams::from_map(entries).or_value_error()?;
    let recorder = Arc::new(ProbeRecorder::new(
        step.headers.clone(),
        step.failure.clone(),
        step.raw_request.clone(),
    ));
    let resolver = Arc::new(ScriptedResolver::new(step.resolution.clone()));
    let entry = HealthCheckEntry {
        backend: Arc::clone(&recorder),
        cheapest: Arc::new(ScriptedCheapest {
            models: step.cheapest.clone(),
        }),
        documents: Arc::new(HealthCheckDocuments),
        resolver: Arc::clone(&resolver),
        catalog: Arc::new(ScriptedCatalog::new(&step.catalog)),
        redactor: Arc::new(Redactor::new(16).or_config_error()?),
        redact_secrets: true,
        clock: || {
            DateTime::parse_from_rfc3339(NOW)
                .unwrap_or_else(|_| DateTime::UNIX_EPOCH.fixed_offset())
        },
        next_id: || FIXED_ID.to_owned(),
    };
    let outcome = entry
        .check_health(
            params,
            step.mode.clone(),
            step.prompt.clone(),
            step.inputs.clone(),
        )
        .await;
    Ok(step_view(&resolver, &recorder, &outcome))
}

fn step_view(
    resolver: &ScriptedResolver,
    recorder: &ProbeRecorder,
    outcome: &HealthCheckOutcome,
) -> Value {
    json!({
        "resolution": resolution_view(resolver),
        "probe": probe_view(recorder),
        "logging": logging_view(recorder),
        "outcome": outcome_view(outcome),
    })
}

fn resolution_view(resolver: &ScriptedResolver) -> Value {
    let requested = resolver
        .requested
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let Some(call) = requested.as_ref() else {
        return Value::Null;
    };
    json!({
        "model": call.model.clone(),
        "custom_llm_provider": call.custom_llm_provider.clone(),
        "api_base": call.api_base.clone(),
        "api_key": call.api_key.clone(),
    })
}

fn probe_view(recorder: &ProbeRecorder) -> Value {
    let call = recorder.call.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(record) = call.as_ref() else {
        return Value::Null;
    };
    json!({"name": record.name, "arguments": without_metadata(record.arguments.clone())})
}

fn without_metadata(mut arguments: Value) -> Value {
    if let Value::Object(entries) = &mut arguments {
        entries.remove("litellm_metadata");
    }
    arguments
}

fn logging_view(recorder: &ProbeRecorder) -> Value {
    let call = recorder.call.lock().unwrap_or_else(PoisonError::into_inner);
    call.as_ref()
        .and_then(|record| record.logging.as_ref())
        .map_or(Value::Null, logged_fields)
}

fn logged_fields(logging: &Arc<dyn HealthProbeLogging>) -> Value {
    let Some(state) = logging.logging_state() else {
        return Value::Null;
    };
    let locked = state.lock().unwrap_or_else(PoisonError::into_inner);
    json!({
        "model": locked.model.clone(),
        "messages": locked.messages.clone(),
        "stream": locked.stream,
        "call_type": locked.call_type.clone(),
        "log_raw_request_response": locked.request_logging.log_raw_request_response,
        "raw_request_only": locked.request_logging.raw_request_only,
        "call_id_uuid4": uuid4(&locked.litellm_call_id),
        "function_id_uuid4": uuid4(&locked.function_id),
        "trace_id_uuid4": uuid4(&locked.litellm_trace_id),
    })
}

fn outcome_view(outcome: &HealthCheckOutcome) -> Value {
    match outcome {
        HealthCheckOutcome::Healthy(response) => json!({"kind": "healthy", "headers": response}),
        HealthCheckOutcome::Failed(failure) => failure_view(failure),
    }
}

fn failure_view(failure: &HealthCheckFailure) -> Value {
    let (head, stack) = split_error(&failure.error);
    json!({
        "kind": "failed",
        "error_head": head,
        "has_stack": stack.is_some(),
        "stack_length": stack.map_or(0_usize, |text| text.chars().count()),
        "raw_request": failure.raw_request.as_ref(),
    })
}

fn split_error(error: &str) -> (&str, Option<&str>) {
    if let Some((head, stack)) = error.split_once(STACK_SEPARATOR) {
        return (head, Some(stack));
    }
    if let Some((head, stack)) = error.split_once(MISSING_MODE_SEPARATOR) {
        return (head, Some(stack));
    }
    (error, None)
}

fn uuid4(text: &str) -> bool {
    text.chars().count() == 36
        && text
            .chars()
            .enumerate()
            .all(|(index, character)| match index {
                8 | 13 | 18 | 23 => character == '-',
                14 => character == '4',
                19 => matches!(character, '8' | '9' | 'a' | 'b'),
                _ => character.is_ascii_digit() || matches!(character, 'a'..='f'),
            })
}
