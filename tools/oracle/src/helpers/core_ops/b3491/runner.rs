use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;

use betterlite_core::api::interactions::contract::records::Params;
use betterlite_core::api::interactions::streaming::async_iterator::InteractionsStreamIterator;
use betterlite_core::api::interactions::streaming::error::InteractionStreamError;
use betterlite_core::api::interactions::streaming::records::{
    InteractionsStreamSetup, SharedApiBase,
};
use betterlite_core::api::interactions::streaming::sync_iterator::SyncInteractionsStreamIterator;
use betterlite_interactions::types::response::InteractionsStreamChunk;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use indexmap::IndexMap;
use serde_json::{Value, json};

use super::config::FixtureConfig;
use super::fixtures::{
    ApiBaseContexts, DispatchOutput, FakeAsyncResponse, FakeSyncResponse, NoHooks,
    RecordingLogging, api_resolver,
};
use super::{Input, Mode};

pub(crate) fn run(input: &Input) -> crate::helpers::HelperResult {
    let catalog = ModelCatalog::default();
    let known = BTreeSet::new();
    let custom = BTreeSet::new();
    let environment = BTreeMap::new();
    let contexts = ApiBaseContexts {
        resolution: ResolutionContext {
            catalog: &catalog,
            known_model_names: &known,
            custom_providers: &custom,
            environment: &environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        },
        aliases: BTreeMap::new(),
        hooks: NoHooks,
    };
    let config = FixtureConfig;
    let resolver = api_resolver(&contexts);
    let mut logging = RecordingLogging::new(call_details(input));
    let output = match input.mode {
        Mode::Async => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(runtime_failure)?;
            runtime.block_on(collect_async(input, &mut logging, &config, &resolver))?
        }
        Mode::Sync => {
            let mut iterator = SyncInteractionsStreamIterator::new(
                FakeSyncResponse::new(&input.lines),
                InteractionsStreamSetup {
                    model: input.model.as_deref(),
                    config: &config,
                    logging: &mut logging,
                    litellm_metadata: None,
                    custom_llm_provider: None,
                    api_base: &resolver,
                },
            )
            .map_err(failure)?;
            let events = std::iter::from_fn(|| iterator.next_chunk())
                .collect::<Result<Vec<_>, _>>()
                .map_err(failure)?;
            let finished = iterator.base.finished;
            let completed = iterator.base.completed_response.clone();
            output(events, finished, completed, logging.records())
        }
    };
    Ok(output)
}

async fn collect_async(
    input: &Input,
    logging: &mut RecordingLogging,
    config: &FixtureConfig,
    resolver: &SharedApiBase<'_>,
) -> crate::helpers::HelperResult {
    let mut iterator = InteractionsStreamIterator::new(
        FakeAsyncResponse::new(&input.lines),
        InteractionsStreamSetup {
            model: input.model.as_deref(),
            config,
            logging: &mut *logging,
            litellm_metadata: None,
            custom_llm_provider: None,
            api_base: resolver,
        },
    )
    .map_err(failure)?;
    let mut events = Vec::new();
    while let Some(item) = iterator.next_chunk().await {
        events.push(item.map_err(failure)?);
    }
    let finished = iterator.base.finished;
    let completed = iterator.base.completed_response.clone();
    Ok(output(events, finished, completed, logging.records()))
}

fn output(
    events: Vec<InteractionsStreamChunk>,
    finished: bool,
    completed: Option<InteractionsStreamChunk>,
    dispatches: Vec<DispatchOutput>,
) -> Value {
    json!({
        "events": events,
        "finished": finished,
        "completed": completed,
        "dispatches": dispatches,
    })
}

fn call_details(input: &Input) -> Params {
    match &input.litellm_params {
        None => Params::new(),
        Some(params) => {
            let fields: IndexMap<String, JsonValue> = params
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect();
            Params::from([(String::from("litellm_params"), JsonValue::Object(fields))])
        }
    }
}

fn failure<SourceError, ConfigError>(
    error: InteractionStreamError<SourceError, ConfigError>,
) -> Box<crate::helpers::HelperError>
where
    SourceError: Error + 'static,
    ConfigError: Error + 'static,
{
    Box::new(crate::helpers::HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn runtime_failure(source: std::io::Error) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("RuntimeError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: source.to_string(),
    })
}
