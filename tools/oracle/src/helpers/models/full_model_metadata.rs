use std::error::Error;
use std::future::Future;
use std::sync::{Arc, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::cache::{MetadataCaches, ModelInfoCache};
use betterlite_models::full_model_metadata::error::{FullMetadataError, ModelRecordError};
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::full_model_metadata::record::ModelInfoHandle;
use betterlite_models::full_model_metadata::{
    FullMetadataSources, HelperQuery, HelperSources, get_model_info, get_model_info_helper_cached,
};
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::MetadataRequest;
use betterlite_models::model_metadata_resolution::error::ModelMetadataError;
use betterlite_models::model_metadata_resolution::hf_config::HfConfigClient;
use betterlite_models::params::capabilities::ModelProjection;
use betterlite_models::provider_metadata_factory::selection::MetadataQuery;
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

mod fixture;
mod input;
mod sources;
mod tests;

use fixture::{FixtureContext, FixtureError, Harness};
use input::{DynamicScript, Input, ParameterScript, Step};
use sources::{CallLog, NoConfig, ProviderSource, RecordingDynamic, ScriptedSource};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let fixture = FixtureContext::load(&input.fixture)
            .map_err(|error| helper_error("FixtureError", error.to_string()))?;
        block_on(run_steps(fixture, input))?
    }
}

struct State {
    keys: ModelCostKeys,
    caches: MetadataCaches,
    dynamic: RecordingDynamic,
    hf: HfConfigClient<NoConfig>,
    log: Arc<CallLog>,
    records: Vec<ModelInfoHandle>,
}

impl State {
    fn new(maxsize: i64, script: Option<DynamicScript>, log: Arc<CallLog>) -> Self {
        Self {
            keys: ModelCostKeys::default(),
            caches: MetadataCaches {
                full: ModelInfoCache::new(maxsize),
                helper: ModelInfoCache::new(maxsize),
            },
            dynamic: RecordingDynamic {
                script,
                log: Arc::clone(&log),
            },
            hf: HfConfigClient {
                client: NoConfig,
                base_url: String::new(),
            },
            log,
            records: Vec::new(),
        }
    }
}

struct Call<'call> {
    model: &'call str,
    custom_llm_provider: Option<&'call str>,
    api_base: Option<&'call str>,
    api_key: Option<&'call str>,
    parameters: Option<&'call ParameterScript>,
}

struct HelperCall<'call> {
    model: &'call str,
    custom_llm_provider: Option<&'call str>,
    api_base: Option<&'call str>,
}

async fn run_steps(mut fixture: FixtureContext, input: &Input) -> Result<Value, Box<HelperError>> {
    let log = Arc::new(CallLog::default());
    let mut state = State::new(
        input.fixture.cache_maxsize.unwrap_or(64),
        input.fixture.dynamic.clone(),
        log,
    );
    let mut observations = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        let start = state.log.len();
        observations.push(run_step(&mut fixture, &mut state, step, start).await?);
    }
    Ok(Value::Array(observations))
}

async fn run_step(
    fixture: &mut FixtureContext,
    state: &mut State,
    step: &Step,
    start: usize,
) -> Result<Value, Box<HelperError>> {
    match step {
        Step::Call { .. } => call_step_for(fixture, state, step, start).await,
        Step::HelperCall { .. } => helper_step_for(fixture, state, step, start).await,
        Step::Mutate {
            record,
            field,
            value,
        } => mutate_step(state, *record, field, value),
        Step::Invalidate => Ok(invalidate_step(state)),
        Step::Clear => Ok(clear_step(state)),
        Step::Replace { key, entry } => replace_step(fixture, key, entry),
    }
}

async fn call_step_for(
    fixture: &FixtureContext,
    state: &mut State,
    step: &Step,
    start: usize,
) -> Result<Value, Box<HelperError>> {
    let Step::Call {
        model,
        custom_llm_provider,
        api_base,
        api_key,
        parameters,
    } = step
    else {
        return Err(helper_error("ValueError", "step is not a call".to_owned()));
    };
    let harness = fixture
        .harness()
        .map_err(|error| helper_error("FixtureError", error.to_string()))?;
    let call = Call {
        model,
        custom_llm_provider: custom_llm_provider.as_deref(),
        api_base: api_base.as_deref(),
        api_key: api_key.as_deref(),
        parameters: parameters.as_ref(),
    };
    call_step(&harness, state, &call, start).await
}

async fn helper_step_for(
    fixture: &FixtureContext,
    state: &mut State,
    step: &Step,
    start: usize,
) -> Result<Value, Box<HelperError>> {
    let Step::HelperCall {
        model,
        custom_llm_provider,
        api_base,
    } = step
    else {
        return Err(helper_error(
            "ValueError",
            "step is not a helper call".to_owned(),
        ));
    };
    let harness = fixture
        .harness()
        .map_err(|error| helper_error("FixtureError", error.to_string()))?;
    let call = HelperCall {
        model,
        custom_llm_provider: custom_llm_provider.as_deref(),
        api_base: api_base.as_deref(),
    };
    helper_step(&harness, state, &call, start).await
}

async fn call_step(
    harness: &Harness<'_>,
    state: &mut State,
    call: &Call<'_>,
    start: usize,
) -> Result<Value, Box<HelperError>> {
    let api_key = call.api_key.map(|key| SecretString::from(key.to_owned()));
    let request = MetadataRequest {
        model: call.model,
        custom_llm_provider: call.custom_llm_provider,
        api_base: call.api_base,
        api_key: api_key.as_ref(),
    };
    match call.parameters.cloned() {
        Some(ParameterScript::B175 {
            call_type,
            config,
            capabilities,
            base_model,
            base_capabilities,
        }) => {
            let base = base_model.as_deref().map(|model| ModelProjection {
                model,
                capabilities: base_capabilities.as_ref().unwrap_or(&capabilities),
            });
            let source = ProviderSource {
                resolution: &harness.resolution,
                call_type,
                config,
                capabilities: &capabilities,
                base_model: base,
                log: Arc::clone(&state.log),
            };
            call_with(&source, harness, state, &request, start).await
        }
        script => {
            let source = ScriptedSource {
                script: script.unwrap_or(ParameterScript::Absent),
                log: Arc::clone(&state.log),
            };
            call_with(&source, harness, state, &request, start).await
        }
    }
}

async fn call_with<Parameters>(
    parameters: &Parameters,
    harness: &Harness<'_>,
    state: &mut State,
    request: &MetadataRequest<'_>,
    start: usize,
) -> Result<Value, Box<HelperError>>
where
    Parameters: SupportedParameterSource,
{
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
        parameters,
    };
    let answer = get_model_info(request, &sources, &mut state.keys, &state.caches).await;
    Ok(observation(state, answer, start, "call"))
}

async fn helper_step(
    harness: &Harness<'_>,
    state: &mut State,
    call: &HelperCall<'_>,
    start: usize,
) -> Result<Value, Box<HelperError>> {
    let query = HelperQuery {
        model: call.model,
        custom_llm_provider: call.custom_llm_provider,
        api_base: call.api_base,
    };
    let environment = harness.environment();
    let sources = HelperSources {
        environment: &environment,
        dynamic: &state.dynamic,
        hf: &state.hf,
    };
    let answer =
        get_model_info_helper_cached(&query, &sources, &mut state.keys, &state.caches.helper).await;
    Ok(observation(state, answer, start, "helper_call"))
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
    match answer {
        Ok(handle) => {
            let same_as = state
                .records
                .iter()
                .position(|previous| Arc::ptr_eq(previous, &handle));
            let record = state.records.len();
            let fields = record_fields(&handle);
            state.records.push(handle);
            json!({
                "op": op,
                "result": fields,
                "record": record,
                "same_as": same_as,
                "calls": calls,
                "caches": caches_json(&state.caches),
            })
        }
        Err(error) => json!({
            "op": op,
            "result": {"raised": raised(&error)},
            "calls": calls,
            "caches": caches_json(&state.caches),
        }),
    }
}

fn record_fields(handle: &ModelInfoHandle) -> Value {
    serde_json::to_value(&*handle.lock().unwrap_or_else(PoisonError::into_inner))
        .unwrap_or(Value::Null)
}

fn mutate_step(
    state: &State,
    record: usize,
    field: &str,
    value: &Value,
) -> Result<Value, Box<HelperError>> {
    let handle = state
        .records
        .get(record)
        .ok_or_else(|| helper_error("IndexError", format!("no record {record}")))?;
    let converted = serde_json::from_value::<Json>(value.clone())
        .map_err(|error| helper_error("ValueError", error.to_string()))?;
    let mut info = handle.lock().unwrap_or_else(PoisonError::into_inner);
    drop(info.fields.insert(field.to_owned(), converted));
    let current = info.fields.get(field).map_or(Value::Null, |present| {
        serde_json::to_value(present).unwrap_or(Value::Null)
    });
    drop(info);
    Ok(json!({
        "op": "mutate",
        "record": record,
        "field": field,
        "value": current,
    }))
}

fn invalidate_step(state: &mut State) -> Value {
    state
        .keys
        .invalidate(&mut state.caches.full, &mut state.caches.helper);
    json!({
        "op": "invalidate",
        "generation": state.keys.generation,
        "caches": caches_json(&state.caches),
    })
}

fn clear_step(state: &State) -> Value {
    state.caches.full.cache_clear();
    json!({
        "op": "clear",
        "caches": caches_json(&state.caches),
    })
}

fn replace_step(
    fixture: &mut FixtureContext,
    key: &str,
    entry: &Value,
) -> Result<Value, Box<HelperError>> {
    fixture
        .replace(key, entry)
        .map_err(|error| helper_error("FixtureError", error.to_string()))?;
    Ok(json!({"op": "replace", "key": key}))
}

fn caches_json(caches: &MetadataCaches) -> Value {
    json!({
        "full": cache_json(&caches.full),
        "helper": cache_json(&caches.helper),
    })
}

fn cache_json(cache: &ModelInfoCache) -> Value {
    let info = cache.cache_info();
    json!({
        "hits": info.hits,
        "misses": info.misses,
        "maxsize": info.maxsize,
        "currsize": info.currsize,
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

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn block_on<FutureType>(future: FutureType) -> Result<FutureType::Output, Box<HelperError>>
where
    FutureType: Future,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| {
            helper_error("FixtureError", FixtureError::Runtime { source }.to_string())
        })?;
    Ok(runtime.block_on(future))
}
