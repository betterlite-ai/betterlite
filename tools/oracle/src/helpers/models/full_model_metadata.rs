use std::future::Future;
use std::sync::Arc;

use betterlite_models::full_model_metadata::cache::{MetadataCaches, ModelInfoCache};
use betterlite_models::full_model_metadata::record::ModelInfoHandle;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::hf_config::HfConfigClient;
use serde_json::Value;

use crate::helpers::{HelperError, HelperResult};

mod fixture;
mod input;
mod sources;
mod steps;
mod tests;

use fixture::FixtureContext;
use input::{Input, Step};
use sources::{CallLog, NoConfig, RecordingDynamic};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let fixture = FixtureContext::load(&input.fixture)
            .map_err(|error| error_box("FixtureError", error.to_string()))?;
        block_on(run_steps(fixture, input))?
    }
}

pub(crate) struct State {
    pub(crate) keys: ModelCostKeys,
    pub(crate) caches: MetadataCaches,
    pub(crate) dynamic: RecordingDynamic,
    pub(crate) hf: HfConfigClient<NoConfig>,
    pub(crate) log: Arc<CallLog>,
    pub(crate) records: Vec<ModelInfoHandle>,
}

impl State {
    fn new(maxsize: i64, script: Option<Value>, log: Arc<CallLog>) -> Self {
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

async fn run_steps(mut fixture: FixtureContext, input: &Input) -> Result<Value, Box<HelperError>> {
    let log = Arc::new(CallLog::default());
    let mut state = State::new(
        input.fixture.cache_maxsize,
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
        Step::Call { .. } => steps::call_step_for(fixture, state, step, start).await,
        Step::HelperCall { .. } => steps::helper_step_for(fixture, state, step, start).await,
        Step::Mutate {
            record,
            field,
            value,
        } => steps::mutate_step(state, *record, field, value),
        Step::Invalidate => Ok(steps::invalidate_step(state)),
        Step::Replace { key, entry } => steps::replace_step(fixture, key, entry),
    }
}

fn error_box(class: &str, message: String) -> Box<HelperError> {
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
        .map_err(|source| error_box("FixtureError", source.to_string()))?;
    Ok(runtime.block_on(future))
}
