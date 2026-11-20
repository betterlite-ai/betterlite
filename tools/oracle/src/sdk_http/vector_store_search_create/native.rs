use std::collections::BTreeMap;

use betterlite_config::api::vector_store_search::Settings;
use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::vector_search_execution::config::Params;
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;
use betterlite_core::api::vector_store_search::error::{
    SdkFailure, VectorStoreSearchError, failure as project_failure,
};
use betterlite_core::api::vector_store_search::service::{
    CreateCall, SdkDeps, SearchCall, create as create_vector_store, search as search_vector_store,
};
use betterlite_error::envelope::WireField;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::super::result::{Failure, Nullable};
use super::support::{
    Assets, Configs, Executors, Filters, Inline, Source, into_index, registry_of, search_optionals,
    source,
};
use super::{Call, Context, CreateArgs, Mode, Operation, Outcome, Run, RunOutcome};

// The production service over the mock's base: the same config sections, filters and pooled client a request would build
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let config = ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let settings = Settings::load(env).map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    let origin = source(&config, env)?;
    let shared = Shared {
        configs: Configs::new(&config),
        filters: Filters,
        executors: Executors,
        signing: Inline,
        source: &origin,
        registry: call.registry.as_ref().map(registry_of),
        settings,
    };
    let assets = Assets::new()?;
    let mut runs = Vec::with_capacity(call.runs.len());
    for run in &call.runs {
        runs.push(run_once(run, api_base, &assets, &shared).await?);
    }
    Ok(SdkResult::VectorStoreSearchCreate {
        value: Box::new(Outcome { runs }),
    })
}

// The parts a run lends to the core service, built once for the whole call
struct Shared<'source> {
    configs: Configs,
    filters: Filters,
    executors: Executors,
    signing: Inline,
    source: &'source Source,
    registry: Option<VectorStoreRegistry>,
    settings: Settings,
}

async fn run_once(
    run: &Run,
    api_base: &str,
    assets: &Assets,
    shared: &Shared<'_>,
) -> Result<RunOutcome, SdkHttpError> {
    let call_type = match run.operation {
        Operation::Create => "vector_store_create",
        Operation::Search => "vector_store_search",
    };
    let mut state = assets.state(call_type)?;
    let pre_post = assets.env();
    let mut log = LogContext {
        state: &mut state,
        env: &pre_post,
    };
    let deps = SdkDeps {
        configs: &shared.configs,
        filters: &shared.filters,
        executors: &shared.executors,
        signing: &shared.signing,
        registry: shared.registry.as_ref(),
        source: shared.source,
        supplied: None,
        settings: shared.settings,
    };
    let kwargs = kwargs_of(run, api_base);
    let answer = match run.operation {
        Operation::Create => create_run(run, &deps, &kwargs, &mut log).await,
        Operation::Search => search_run(run, &deps, &kwargs, &mut log).await,
    };
    // update_from_kwargs is what names the provider; a call that failed before it has no context to record
    let context = state.custom_llm_provider.as_ref().map(|provider| Context {
        model: state.model.clone(),
        optional_params: state
            .optional_params
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        custom_llm_provider: provider.clone(),
        litellm_call_id: state
            .litellm_params
            .get("litellm_call_id")
            .cloned()
            .unwrap_or(Json::Null),
        vector_store_id: state
            .litellm_params
            .get("vector_store_id")
            .cloned()
            .unwrap_or(Json::Null),
    });
    let outcome = match answer {
        Ok(value) => RunOutcome {
            operation: run.operation,
            mode: run.mode,
            result: Some(value),
            failure: None,
            context,
        },
        Err(error) => RunOutcome {
            operation: run.operation,
            mode: run.mode,
            result: None,
            failure: Some(sdk_failure(&error, assets.redactor())?),
            context,
        },
    };
    Ok(outcome)
}

async fn create_run(
    run: &Run,
    deps: &SdkDeps<'_, Source>,
    kwargs: &Params,
    log: &mut LogContext<'_, '_>,
) -> Result<Json, VectorStoreSearchError> {
    let args = missing(run.create.as_ref(), "create")?;
    let optional = create_optionals(args);
    let extra_body: Option<IndexMap<String, Json>> = run.extra_body.as_ref().map(into_index);
    let call = CreateCall {
        optional: &optional,
        kwargs,
        extra_headers: run.extra_headers.as_ref(),
        extra_body: extra_body.as_ref(),
        timeout: run.timeout_seconds,
        custom_llm_provider: run.custom_llm_provider.as_deref(),
        entry: run.mode.entry(),
    };
    create_vector_store(deps, &call, log).await
}

async fn search_run(
    run: &Run,
    deps: &SdkDeps<'_, Source>,
    kwargs: &Params,
    log: &mut LogContext<'_, '_>,
) -> Result<Json, VectorStoreSearchError> {
    let args = missing(run.search.as_ref(), "search")?;
    let optional = search_optionals(args);
    let call = SearchCall {
        vector_store_id: &args.vector_store_id,
        query: &args.query,
        optional: &optional,
        kwargs,
        extra_headers: run.extra_headers.as_ref(),
        extra_body: run.extra_body.as_ref(),
        timeout: run.timeout_seconds,
        custom_llm_provider: run.custom_llm_provider.as_deref(),
        executor: None,
        entry: run.mode.entry(),
    };
    search_vector_store(deps, &call, log).await
}

fn missing<'args, T>(held: Option<&'args T>, operation: &str) -> Result<&'args T, SdkHttpError> {
    held.ok_or_else(|| {
        SdkHttpError::Invalid(format!(
            "a {operation} run carries the {operation} arguments"
        ))
    })
}

// GenericLiteLLMParams(**kwargs) plus the base the mock answered on
fn kwargs_of(run: &Run, api_base: &str) -> Params {
    let mut kwargs = Params::new();
    kwargs.insert("api_base".to_owned(), Json::Text(api_base.to_owned()));
    if let Some(key) = &run.options.api_key {
        kwargs.insert("api_key".to_owned(), Json::Text(key.clone()));
    }
    kwargs.insert(
        "litellm_call_id".to_owned(),
        Json::Text(run.options.litellm_call_id.clone()),
    );
    if let Some(mock) = &run.options.mock_response {
        kwargs.insert("mock_response".to_owned(), mock.clone());
    }
    kwargs
}

// VectorStoreCreateOptionalRequestParams as the case spelled it: only the keys the caller set
fn create_optionals(args: &CreateArgs) -> Params {
    let mut optional = Params::new();
    if let Some(name) = &args.name {
        optional.insert("name".to_owned(), Json::Text(name.clone()));
    }
    if let Some(file_ids) = &args.file_ids {
        optional.insert(
            "file_ids".to_owned(),
            Json::List(file_ids.iter().map(|id| Json::Text(id.clone())).collect()),
        );
    }
    if let Some(policy) = &args.expires_after {
        optional.insert("expires_after".to_owned(), Json::Object(into_index(policy)));
    }
    if let Some(strategy) = &args.chunking_strategy {
        optional.insert(
            "chunking_strategy".to_owned(),
            Json::Object(into_index(strategy)),
        );
    }
    if let Some(metadata) = &args.metadata {
        optional.insert(
            "metadata".to_owned(),
            Json::Object(metadata_index(metadata)),
        );
    }
    optional
}

fn metadata_index(metadata: &BTreeMap<String, String>) -> IndexMap<String, Json> {
    metadata
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}

// The exception exception_type raises, as its callers can observe it
fn sdk_failure(
    error: &VectorStoreSearchError,
    redactor: &Redactor,
) -> Result<Failure, SdkHttpError> {
    let projected: SdkFailure = project_failure(error, redactor);
    let status = projected.status.ok_or_else(|| {
        SdkHttpError::Invalid("the mapped failure carries no status code".to_owned())
    })?;
    Ok(Failure {
        class: projected.class,
        status,
        kind: field(projected.kind),
        param: field(projected.param),
        code: field(projected.code),
        message: projected.message,
    })
}

// mirrors getattr(e, field, "None"): absent, present and null, or text
fn field(wire: WireField) -> Nullable {
    match wire {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text)),
    }
}
