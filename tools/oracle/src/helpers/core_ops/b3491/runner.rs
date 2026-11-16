use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::io;
use std::iter::from_fn;

use betterlite_core::api::interactions::contract::records::Params;
use betterlite_core::api::interactions::streaming::async_iterator::InteractionsStreamIterator;
use betterlite_core::api::interactions::streaming::base::InteractionsStreamBase;
use betterlite_core::api::interactions::streaming::error::InteractionStreamError;
use betterlite_core::api::interactions::streaming::records::InteractionsStreamSetup;
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
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use tokio::runtime::Builder;

type Setup<'setup> = InteractionsStreamSetup<'setup, FixtureConfig, RecordingLogging>;
type AsyncStream<'base> =
    InteractionsStreamIterator<'base, FakeAsyncResponse, FixtureConfig, RecordingLogging>;
type SyncStream<'base> =
    SyncInteractionsStreamIterator<'base, FakeSyncResponse, FixtureConfig, RecordingLogging>;

enum Streams<'base> {
    Async(AsyncStream<'base>),
    Sync(SyncStream<'base>),
}

pub(crate) fn run(input: &Input) -> HelperResult {
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
    let setup = InteractionsStreamSetup {
        model: input.model.as_deref(),
        config: &config,
        logging: &mut logging,
        litellm_metadata: None,
        custom_llm_provider: None,
        api_base: &resolver,
    };
    drive(open_stream(input, setup)?)
}

fn open_stream<'setup>(
    input: &Input,
    setup: Setup<'setup>,
) -> Result<Streams<'setup>, Box<HelperError>> {
    match input.mode {
        Mode::Async => {
            let response = FakeAsyncResponse::new(&input.lines);
            InteractionsStreamIterator::new(response, setup)
                .map(Streams::Async)
                .map_err(|error| failure(&error))
        }
        Mode::Sync => {
            let response = FakeSyncResponse::new(&input.lines);
            SyncInteractionsStreamIterator::new(response, setup)
                .map(Streams::Sync)
                .map_err(|error| failure(&error))
        }
    }
}

fn drive(streams: Streams<'_>) -> HelperResult {
    match streams {
        Streams::Async(mut iterator) => {
            let runtime = Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| runtime_failure(&error))?;
            runtime.block_on(collect_async(&mut iterator))
        }
        Streams::Sync(mut iterator) => {
            let events = from_fn(|| iterator.next_chunk())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| failure(&error))?;
            Ok(finish(&iterator.base, &events))
        }
    }
}

async fn collect_async(iterator: &mut AsyncStream<'_>) -> HelperResult {
    let mut events = Vec::new();
    while let Some(item) = iterator.next_chunk().await {
        events.push(item.map_err(|error| failure(&error))?);
    }
    Ok(finish(&iterator.base, &events))
}

fn finish(
    base: &InteractionsStreamBase<'_, FixtureConfig, RecordingLogging>,
    events: &[InteractionsStreamChunk],
) -> Value {
    let dispatches = base.logging.records();
    output(
        events,
        base.finished,
        base.completed_response.as_ref(),
        &dispatches,
    )
}

fn output(
    events: &[InteractionsStreamChunk],
    finished: bool,
    completed: Option<&InteractionsStreamChunk>,
    dispatches: &[DispatchOutput],
) -> Value {
    json!({
        "events": events,
        "finished": finished,
        "completed": completed,
        "dispatches": dispatches,
    })
}

fn call_details(input: &Input) -> Params {
    let fields: IndexMap<String, JsonValue> = input
        .litellm_params
        .iter()
        .flatten()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    Params::from([(String::from("litellm_params"), JsonValue::Object(fields))])
}

fn failure<SourceError, ConfigError>(
    error: &InteractionStreamError<SourceError, ConfigError>,
) -> Box<HelperError>
where
    SourceError: Error + 'static,
    ConfigError: Error + 'static,
{
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn runtime_failure(source: &io::Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("RuntimeError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: source.to_string(),
    })
}
