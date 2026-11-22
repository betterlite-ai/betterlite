use super::recorder::{FixedCheapest, MissingDocuments, Recorder, UpdateRecorder};
use super::{ModeStep, TRACKED, TrackingStep, WildcardStep};
use betterlite_models::decisions::Json;
use betterlite_server::health_probe_modes::error::HealthProbeError;
use betterlite_server::health_probe_modes::logging::HealthProbeLogging;
use betterlite_server::health_probe_modes::params::{HealthProbeParams, Param};
use betterlite_server::health_probe_modes::plan::{HealthProbeArguments, plan_health_probe};
use betterlite_server::health_probe_modes::run::run_health_probe;
use betterlite_server::health_probe_modes::tracking::add_tracking_information;
use betterlite_server::health_probe_modes::wildcard::probe_wildcard_model;
use indexmap::IndexMap;
use serde_json::{Map, Value, json};
use std::sync::{Arc, Mutex, PoisonError};
pub(super) async fn mode_step(step: &ModeStep) -> Value {
    let entries: IndexMap<String, Value> = step.params.clone().into_iter().collect();
    let mut params = match HealthProbeParams::from_map(entries) {
        Ok(params) => params,
        Err(error) => {
            return json!({"error": {"kind": "other", "detail": error.to_string()}});
        }
    };
    let updates = Arc::new(UpdateRecorder::default());
    if step.logging {
        let logging: Arc<dyn HealthProbeLogging> = Arc::<UpdateRecorder>::clone(&updates);
        params.litellm_logging_obj = Some(logging);
    }
    let arguments = HealthProbeArguments {
        prompt: step.prompt.clone(),
        input: step.inputs.clone(),
    };
    let Some(plan) = plan_health_probe(
        &step.mode,
        &step.model,
        &step.custom_llm_provider,
        params,
        arguments,
    ) else {
        return json!({"error": {"kind": "mode", "detail": &step.mode}});
    };
    let calls = Recorder {
        calls: Mutex::new(Vec::new()),
        failure: step.failure.clone(),
    };
    match run_health_probe(plan, &calls, &MissingDocuments).await {
        Ok(_) => success_outcome(step, &calls, &updates),
        Err(error) => mode_error(step, &error),
    }
}
pub(super) async fn wildcard_step(step: &WildcardStep) -> Value {
    let entries: IndexMap<String, Value> = step.params.clone().into_iter().collect();
    let params = match HealthProbeParams::from_map(entries) {
        Ok(params) => params,
        Err(error) => {
            return json!({"error": {"kind": "other", "detail": error.to_string()}});
        }
    };
    let calls = Recorder {
        calls: Mutex::new(Vec::new()),
        failure: None,
    };
    let cheapest = FixedCheapest {
        models: step.cheapest.clone(),
    };
    let reply = probe_wildcard_model(
        &step.model,
        &step.custom_llm_provider,
        params,
        &cheapest,
        &calls,
    )
    .await;
    match reply {
        Ok(_) => {
            let call = calls
                .calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .first()
                .cloned();
            json!({"call": call})
        }
        Err(error) => json!({"error": {"kind": "no_model", "detail": error.to_string()}}),
    }
}
pub(super) fn tracking_step(step: &TrackingStep) -> Value {
    let entries: IndexMap<String, Value> = step.params.clone().into_iter().collect();
    let params = match HealthProbeParams::from_map(entries) {
        Ok(params) => params,
        Err(error) => {
            return json!({"error": {"kind": "other", "detail": error.to_string()}});
        }
    };
    let tracked = match add_tracking_information(params) {
        Ok(tracked) => tracked,
        Err(error) => {
            return json!({"error": {"kind": "other", "detail": error.to_string()}});
        }
    };
    let Param::Value(Json::Object(metadata)) = tracked.litellm_metadata else {
        return json!({"error": {"kind": "other", "detail": "the tracking metadata is not an object"}});
    };
    let mut projected = Map::new();
    for key in TRACKED {
        let value = metadata.get(key).map_or(Value::Null, |found| {
            serde_json::to_value(found).unwrap_or(Value::Null)
        });
        projected.insert(key.to_owned(), value);
    }
    Value::Object(projected)
}
fn success_outcome(step: &ModeStep, calls: &Recorder, updates: &UpdateRecorder) -> Value {
    let call = calls
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
        .cloned();
    let logged = updates
        .update
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let result = if step.failure.is_some() {
        "empty"
    } else {
        "ok"
    };
    json!({"call": call, "logging": logged, "result": result})
}
fn mode_error(step: &ModeStep, error: &HealthProbeError) -> Value {
    match error {
        HealthProbeError::RepeatedParameter { key, .. } => {
            json!({"error": {"kind": "repeated", "detail": key}})
        }
        HealthProbeError::EvaluationPayload { field, .. } => {
            json!({"error": {"kind": "payload", "detail": field}})
        }
        HealthProbeError::Backend { .. } => {
            let class = step
                .failure
                .as_ref()
                .map_or("", |failure| failure.class_name.as_str());
            json!({"error": {"kind": "backend", "detail": class}})
        }
        HealthProbeError::ParameterValue { .. }
        | HealthProbeError::NoCheapModel { .. }
        | HealthProbeError::OcrDocumentUnavailable { .. }
        | HealthProbeError::HealthImage { .. }
        | HealthProbeError::Logging { .. }
        | HealthProbeError::Principal { .. }
        | HealthProbeError::PrincipalEncode { .. }
        | HealthProbeError::Tracking { .. }
        | HealthProbeError::TrackingMetadata => {
            json!({"error": {"kind": "other", "detail": error.to_string()}})
        }
    }
}
