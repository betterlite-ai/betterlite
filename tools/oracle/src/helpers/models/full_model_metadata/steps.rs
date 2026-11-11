use std::error::Error;
use std::sync::{Arc, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::cache::MetadataCaches;
use betterlite_models::full_model_metadata::error::{FullMetadataError, ModelRecordError};
use betterlite_models::full_model_metadata::record::ModelInfoHandle;
use betterlite_models::full_model_metadata::{
    FullMetadataSources, HelperQuery, HelperSources, get_model_info, get_model_info_helper_cached,
};
use betterlite_models::model_metadata_resolution::MetadataRequest;
use betterlite_models::model_metadata_resolution::error::ModelMetadataError;
use betterlite_models::provider_metadata_factory::selection::MetadataQuery;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::input::Step;
use super::sources::ScriptedSource;
use super::{FixtureContext, State, error_box};
use crate::helpers::HelperError;

type StepResult = Result<Value, Box<HelperError>>;

pub(super) async fn call_step_for(
    fixture: &FixtureContext,
    state: &mut State,
    step: &Step,
    start: usize,
) -> StepResult {
    let Step::Call {
        model,
        custom_llm_provider,
        api_base,
        api_key,
        parameters,
    } = step
    else {
        return Err(error_box("ValueError", "step is not a call".to_owned()));
    };
    let credential = api_key.as_ref().map(|key| SecretString::from(key.clone()));
    let request = MetadataRequest {
        model,
        custom_llm_provider: custom_llm_provider.as_deref(),
        api_base: api_base.as_deref(),
        api_key: credential.as_ref(),
    };
    let source = ScriptedSource {
        script: parameters.clone(),
        log: Arc::clone(&state.log),
    };
    let harness = fixture.harness();
    let environment = harness.environment();
    let sources = FullMetadataSources {
        helper: HelperSources {
            environment: &environment,
            dynamic: &state.dynamic,
            hf: &state.hf,
        },
        provider_query: MetadataQuery {
            catalog: &harness.fixture.fireworks,
            generation: state.keys.generation,
        },
        parameters: &source,
    };
    let answer = get_model_info(&request, &sources, &mut state.keys, &state.caches).await;
    Ok(observation(state, answer, start, "call"))
}

pub(super) async fn helper_step_for(
    fixture: &FixtureContext,
    state: &mut State,
    step: &Step,
    start: usize,
) -> StepResult {
    match step {
        Step::HelperCall {
            model,
            custom_llm_provider,
            api_base,
        } => {
            let query = HelperQuery {
                model,
                custom_llm_provider: custom_llm_provider.as_deref(),
                api_base: api_base.as_deref(),
            };
            let harness = fixture.harness();
            let environment = harness.environment();
            let sources = HelperSources {
                environment: &environment,
                dynamic: &state.dynamic,
                hf: &state.hf,
            };
            let answer = get_model_info_helper_cached(
                &query,
                &sources,
                &mut state.keys,
                &state.caches.helper,
            )
            .await;
            Ok(observation(state, answer, start, "helper_call"))
        }
        Step::Call { .. } | Step::Mutate { .. } | Step::Invalidate | Step::Replace { .. } => Err(
            error_box("ValueError", "step is not a helper call".to_owned()),
        ),
    }
}

fn observation<ParameterError>(
    state: &mut State,
    answer: Result<ModelInfoHandle, FullMetadataError<ParameterError>>,
    start: usize,
    op: &str,
) -> Value
where
    ParameterError: Error + 'static,
{
    let calls = state.log.since(start);
    let caches = caches_json(&state.caches);
    match answer {
        Ok(handle) => {
            let record = state.records.len();
            let fields = record_fields(&handle);
            state.records.push(handle);
            json!({
                "op": op,
                "result": fields,
                "record": record,
                "calls": calls,
                "caches": caches,
            })
        }
        Err(error) => json!({
            "op": op,
            "result": {"raised": raised(&error)},
            "calls": calls,
            "caches": caches,
        }),
    }
}

fn record_fields(handle: &ModelInfoHandle) -> Value {
    serde_json::to_value(&*handle.lock().unwrap_or_else(PoisonError::into_inner))
        .unwrap_or(Value::Null)
}

pub(super) fn mutate_step(state: &State, record: usize, field: &str, value: &Value) -> StepResult {
    let handle = state
        .records
        .get(record)
        .ok_or_else(|| error_box("IndexError", format!("no record {record}")))?;
    let converted = serde_json::from_value::<Json>(value.clone())
        .map_err(|error| error_box("ValueError", error.to_string()))?;
    let mut info = handle.lock().unwrap_or_else(PoisonError::into_inner);
    drop(info.fields.insert(field.to_owned(), converted));
    drop(info);
    Ok(json!({"op": "mutate", "record": record, "field": field}))
}

pub(super) fn invalidate_step(state: &mut State) -> Value {
    state
        .keys
        .invalidate(&mut state.caches.full, &mut state.caches.helper);
    json!({
        "op": "invalidate",
        "generation": state.keys.generation,
        "caches": caches_json(&state.caches),
    })
}

pub(super) fn replace_step(fixture: &mut FixtureContext, key: &str, entry: &Value) -> StepResult {
    fixture
        .replace(key, entry)
        .map_err(|error| error_box("FixtureError", error.to_string()))?;
    Ok(json!({"op": "replace", "key": key}))
}

fn caches_json(caches: &MetadataCaches) -> Value {
    let full = caches.full.cache_info();
    json!({
        "full": {"hits": full.hits, "misses": full.misses, "maxsize": full.maxsize, "currsize": full.currsize},
        "helper": {"hits": caches.helper.cache_info().hits, "misses": caches.helper.cache_info().misses, "maxsize": caches.helper.cache_info().maxsize, "currsize": caches.helper.cache_info().currsize},
    })
}

const fn raised<ParameterError>(error: &FullMetadataError<ParameterError>) -> &'static str
where
    ParameterError: Error + 'static,
{
    match error {
        FullMetadataError::Parameters { .. } => "parameters",
        FullMetadataError::Metadata(ModelMetadataError::NotMapped { .. }) => "not_mapped",
        FullMetadataError::Metadata(ModelMetadataError::Failed { .. }) => "failed",
        FullMetadataError::Record(ModelRecordError::DuplicateSupportedParameters) => {
            "duplicate_supported_openai_params"
        }
        FullMetadataError::Record(
            ModelRecordError::Json { .. } | ModelRecordError::NotAnObject,
        ) => "record",
    }
}
