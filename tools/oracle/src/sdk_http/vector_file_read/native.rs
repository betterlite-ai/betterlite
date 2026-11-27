use std::collections::BTreeMap;

use betterlite_config::api::vector_file_read::loads_read_timeout;
use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::vector_file_provider::CredentialLookup;
use betterlite_core::api::vector_file_read::error::{
    VectorFileReadError, failure as project_failure,
};
use betterlite_core::api::vector_file_read::service::{ReadCall, SdkDeps};
use betterlite_core::api::vector_file_read::{
    aretrieve, aretrieve_content, retrieve, retrieve_content,
};
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;
use betterlite_error::envelope::WireField;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::super::result::{Failure, Nullable};
use super::support::{Assets, Configs, Source, kwargs_of, registry_of, source};
use super::{Call, Context, Mode, Operation, Outcome, Run, RunOutcome};

// The production wrappers over the mock's base: the same config sections, registry and pooled client a request would build
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
    let request_timeout_seconds =
        loads_read_timeout(env).map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let origin = source(&config, env)?;
    let fixtures = Fixtures {
        configs: Configs::new(&config),
        source: &origin,
        registry: call.registry.as_ref().map(registry_of),
        assets: Assets::new()?,
        request_timeout_seconds,
    };
    let mut runs = Vec::with_capacity(call.runs.len());
    for run in &call.runs {
        runs.push(run_once(run, api_base, &fixtures).await?);
    }
    Ok(SdkResult::VectorFileRead {
        value: Box::new(Outcome { runs }),
    })
}

// The parts a run lends to the core service, built once for the whole call
struct Fixtures<'source> {
    configs: Configs,
    source: &'source Source,
    registry: Option<VectorStoreRegistry>,
    assets: Assets,
    request_timeout_seconds: f64,
}

async fn run_once(
    run: &Run,
    api_base: &str,
    fixtures: &Fixtures<'_>,
) -> Result<RunOutcome, SdkHttpError> {
    let mut state = fixtures.assets.state(run.operation.call_type())?;
    let pre_post = fixtures.assets.env();
    let mut log = LogContext {
        state: &mut state,
        env: &pre_post,
    };
    let deps = deps_of(fixtures);
    let kwargs = kwargs_of(run, api_base);
    let call = ReadCall {
        vector_store_id: &run.vector_store_id,
        file_id: &run.file_id,
        extra_headers: run.extra_headers.as_ref(),
        timeout: run.timeout_seconds.map(f64::from),
        custom_llm_provider: run.custom_llm_provider.as_deref(),
        kwargs: &kwargs,
    };
    let answer = match (run.operation, run.mode) {
        (Operation::Retrieve, Mode::Sync) => retrieve(&deps, &call, &mut log).await,
        (Operation::Retrieve, Mode::Asynchronous) => aretrieve(&deps, &call, &mut log).await,
        (Operation::Content, Mode::Sync) => retrieve_content(&deps, &call, &mut log).await,
        (Operation::Content, Mode::Asynchronous) => aretrieve_content(&deps, &call, &mut log).await,
    };
    let failure = match &answer {
        Ok(_) => None,
        Err(error) => Some(sdk_failure(error, &fixtures.assets.redactor)),
    };
    Ok(RunOutcome {
        operation: run.operation,
        mode: run.mode,
        result: answer.ok(),
        failure,
        context: context_of(&state),
    })
}

fn deps_of<'fixtures>(fixtures: &'fixtures Fixtures<'_>) -> SdkDeps<'fixtures, Source> {
    SdkDeps {
        configs: &fixtures.configs,
        registry: fixtures
            .registry
            .as_ref()
            .map(|registry| registry as &dyn CredentialLookup),
        source: fixtures.source,
        supplied: None,
        request_timeout_seconds: fixtures.request_timeout_seconds,
    }
}

// update_from_kwargs is what names the provider; a call that failed before it has no context to record
fn context_of(state: &LoggingState) -> Option<Context> {
    let provider = state.custom_llm_provider.as_ref()?;
    Some(Context {
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
    })
}

// The exception a caller observes, as the pinned SDK raises it
fn sdk_failure(error: &VectorFileReadError, redactor: &Redactor) -> Failure {
    let projected = project_failure(error, redactor);
    Failure {
        class: projected.class,
        status: projected.status,
        kind: field(projected.kind),
        param: field(projected.param),
        code: field(projected.code),
        message: projected.message,
    }
}

// mirrors getattr(e, field, "None"): absent, present and null, or text
fn field(wire: WireField) -> Nullable {
    match wire {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text)),
    }
}
