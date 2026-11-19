use std::collections::BTreeMap;
use std::mem::take;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_compression::compress::CompressionSkippedReason;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::compression_interception_state::Settings;
use betterlite_core::api::compression_interception::cache::Clock;
use betterlite_core::api::compression_interception::engine::CompressionOutput;
use betterlite_core::api::compression_interception::hook::{CompressionInterception, HookResult};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use super::run::{CALL_ID_FIELD, Diagnostics, GENERATED_ID, ScriptedEngine};
use super::{
    Call, CompressCallRecord, DiagnosticRecord, Outcome, ScriptedResult, SettingsRecord,
    SettingsSource, Step, StepOutcome,
};
use crate::sdk_http::error::SdkHttpError;

#[derive(Serialize)]
struct Document<'document> {
    litellm_settings: &'document IndexMap<String, Json>,
    callback_settings: &'document IndexMap<String, Json>,
}

pub(super) struct Session {
    engine: Arc<ScriptedEngine>,
    clock: Clock,
    now: Arc<Mutex<f64>>,
    pub(super) diagnostics: Diagnostics,
    env: BTreeMap<String, String>,
}

impl Session {
    pub(super) fn new(env: &BTreeMap<String, String>) -> Self {
        let now = Arc::new(Mutex::new(0.0_f64));
        let clock_value = Arc::clone(&now);
        Self {
            engine: Arc::new(ScriptedEngine::default()),
            clock: Arc::new(move || *clock_value.lock().unwrap_or_else(PoisonError::into_inner)),
            now,
            diagnostics: Diagnostics::default(),
            env: env.clone(),
        }
    }

    pub(super) async fn run(&self, call: &Call) -> Result<Outcome, SdkHttpError> {
        let mut interception = None;
        let mut steps = Vec::with_capacity(call.steps.len());
        for step in &call.steps {
            if let Some(source) = &step.settings {
                interception = Some(self.interception(source)?);
            }
            let Some(active) = &interception else {
                return Err(SdkHttpError::Invalid(
                    "the first step names no settings".to_owned(),
                ));
            };
            steps.push(self.run_step(step, active).await?);
        }
        Ok(Outcome { steps })
    }

    fn interception(
        &self,
        source: &SettingsSource,
    ) -> Result<CompressionInterception<ScriptedEngine>, SdkHttpError> {
        let settings = load_settings(source, &self.env)?;
        Ok(CompressionInterception::with_clock(
            settings,
            Arc::clone(&self.engine),
            Arc::clone(&self.clock),
        ))
    }

    fn stage_output(&self, scripted: Option<&ScriptedResult>) -> Result<(), SdkHttpError> {
        let Some(step_result) = scripted else {
            return Ok(());
        };
        let output = scripted_output(step_result)?;
        *self
            .engine
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(output);
        Ok(())
    }

    async fn run_step(
        &self,
        step: &Step,
        interception: &CompressionInterception<ScriptedEngine>,
    ) -> Result<StepOutcome, SdkHttpError> {
        self.stage_output(step.compress.as_ref())?;
        *self.now.lock().unwrap_or_else(PoisonError::into_inner) = step.now;
        let mut request = step.request.clone();
        let previous = request.get(CALL_ID_FIELD).cloned();
        let result = interception
            .pre_call_deployment_hook(&mut request, step.call_type)
            .await
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
        normalize_generated_id(&mut request, previous.as_ref());
        let mut lookup: Vec<String> = interception
            .get_cache(step.lookup.as_deref())
            .into_keys()
            .collect();
        lookup.sort();
        let (compress_calls, diagnostics) = self.drain_engine();
        Ok(StepOutcome {
            settings: SettingsRecord {
                enabled: interception.settings.enabled,
                compression_trigger: interception.settings.compression_trigger,
                compression_target: interception.settings.compression_target,
                embedding_model: interception.settings.embedding_model.clone(),
                embedding_model_params: interception.settings.embedding_model_params.clone(),
            },
            skipped: result == HookResult::Skipped,
            request,
            compress_calls,
            lookup,
            diagnostics,
        })
    }

    fn drain_engine(&self) -> (Vec<CompressCallRecord>, Vec<DiagnosticRecord>) {
        let calls = take(
            &mut *self
                .engine
                .calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        let diagnostics = take(
            &mut *self
                .diagnostics
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        (calls, diagnostics)
    }
}

fn load_settings(
    source: &SettingsSource,
    env: &BTreeMap<String, String>,
) -> Result<Settings, SdkHttpError> {
    let document = serde_json::to_string(&Document {
        litellm_settings: &source.litellm_settings,
        callback_settings: &source.callback_settings,
    })
    .map_err(SdkHttpError::Serialize)?;
    let config = ConfigLoader {
        lookup: env.clone(),
    }
    .load(&document)
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    Ok(config.sections.compression_interception_state)
}

fn scripted_output(scripted: &ScriptedResult) -> Result<CompressionOutput, SdkHttpError> {
    Ok(CompressionOutput {
        messages: scripted.messages.clone(),
        tools: scripted.tools.clone(),
        original_tokens: scripted.original_tokens.clone(),
        compressed_tokens: scripted.compressed_tokens.clone(),
        cache: scripted.cache.clone(),
        skipped_reason: skipped_reason(scripted.skipped_reason.as_deref())?,
    })
}

fn skipped_reason(name: Option<&str>) -> Result<Option<CompressionSkippedReason>, SdkHttpError> {
    match name {
        None => Ok(None),
        Some("below_trigger") => Ok(Some(CompressionSkippedReason::BelowTrigger)),
        Some("invalid_anthropic_tool_sequence") => {
            Ok(Some(CompressionSkippedReason::InvalidAnthropicToolSequence))
        }
        Some(other) => Err(SdkHttpError::Invalid(format!(
            "the case scripts an unknown skipped reason {other}"
        ))),
    }
}

fn normalize_generated_id(request: &mut IndexMap<String, Json>, previous: Option<&Json>) {
    let Some(Json::Text(current)) = request.get(CALL_ID_FIELD) else {
        return;
    };
    let kept = match previous {
        Some(Json::Text(existing)) => existing == current,
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            false
        }
    };
    if current.is_empty() || kept {
        return;
    }
    request.insert(
        CALL_ID_FIELD.to_owned(),
        Json::Text(GENERATED_ID.to_owned()),
    );
}
