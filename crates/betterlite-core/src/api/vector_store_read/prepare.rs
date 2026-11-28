use std::sync::PoisonError;
use std::time::Duration;

use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use indexmap::IndexMap;
use serde_json::Number;

use super::DEFAULT_PROVIDER;
use super::error::{VectorStoreReadError, VectorStoreReadFailure, failure};
use super::service::{ListQuery, Operation, ReadConfig};
use super::{
    Content, TransportRequest, VectorStoreListRequest, VectorStoreReadDeps, VectorStoreReadRequest,
};
use crate::api::vector_store_search::provider::resolve_provider;

pub(crate) struct Target<'deps> {
    pub(crate) operation: Operation,
    // the provider name after a slash was split off, which is what LiteLLM logs and maps with
    pub(crate) name: String,
    pub(crate) provider: ProviderId,
    pub(crate) config: ReadConfig<'deps>,
}

// B174 turns the declared name into a provider and an api_type, then B010's source selects that provider's configuration
pub(crate) fn target<'deps, Client>(
    operation: Operation,
    transport: &TransportRequest<'_>,
    deps: &VectorStoreReadDeps<'deps, Client>,
) -> Result<Target<'deps>, VectorStoreReadFailure> {
    let requested = transport.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER);
    let resolved =
        resolve_provider(operation.label(), transport.custom_llm_provider).map_err(|source| {
            let error = VectorStoreReadError::Resolution(source);
            failure(error, requested, deps.redactor)
        })?;
    let name = resolved.provider.to_string();
    let Some(config) = deps.configs.read_config(&resolved) else {
        let error = VectorStoreReadError::Unconfigured {
            operation,
            provider: name.clone(),
        };
        return Err(failure(error, &name, deps.redactor));
    };
    Ok(Target {
        operation,
        name,
        provider: resolved.provider,
        config,
    })
}

// `timeout or request_timeout`: an explicit zero falls back, and an unusable value becomes a zero deadline no GET reads
pub(crate) fn call_timeout(requested: Option<f64>, configured: f64) -> Duration {
    let seconds = requested
        .filter(|seconds| *seconds != 0.0_f64)
        .unwrap_or(configured);
    Duration::try_from_secs_f64(seconds).unwrap_or(Duration::ZERO)
}

// the operation's own arguments: what pre-call logging reports and what the provider's read takes
#[derive(Clone, Copy)]
pub(crate) enum Arguments<'request> {
    Retrieve(&'request VectorStoreReadRequest<'request>),
    List(&'request VectorStoreListRequest<'request>),
}

impl<'request> Arguments<'request> {
    // the transport the operation's request carried, whose kwargs pre-call logging reads
    const fn transport(&self) -> TransportRequest<'request> {
        match self {
            Self::Retrieve(request) => request.transport,
            Self::List(request) => request.transport,
        }
    }
}

// both operations log the call before the provider's read, and a failure there is raised the way LiteLLM raises it
pub(crate) fn options<'request, Client>(
    deps: &VectorStoreReadDeps<'_, Client>,
    target: &Target<'_>,
    arguments: Arguments<'request>,
) -> Result<Content<'request>, VectorStoreReadFailure> {
    let (content, optional_params) = match arguments {
        Arguments::Retrieve(request) => (
            Content {
                vector_store_id: Some(request.vector_store_id),
                query: ListQuery::default(),
            },
            retrieve_params(request.vector_store_id),
        ),
        Arguments::List(request) => {
            // the signature's defaults: an absent limit and order become 20 and desc, while an explicit null stays absent
            let query = ListQuery {
                after: request.after,
                before: request.before,
                limit: request.limit.unwrap_or(Some(20)),
                order: request.order.unwrap_or(Some("desc")),
            };
            (
                Content {
                    vector_store_id: None,
                    query,
                },
                list_optional_params(query),
            )
        }
    };
    let transport = arguments.transport();
    log_call(
        deps,
        transport.kwargs,
        target.operation.flag(),
        optional_params,
        &target.name,
    )
    .map_err(|error| failure(error, &target.name, deps.redactor))?;
    Ok(content)
}

// retrieve logs the id alone, and the read needs nothing more
fn retrieve_params(vector_store_id: &str) -> IndexMap<String, Json> {
    IndexMap::from([(
        String::from("vector_store_id"),
        Json::Text(vector_store_id.to_owned()),
    )])
}

// every list argument enters the log, each one present even when it is null
fn list_optional_params(query: ListQuery<'_>) -> IndexMap<String, Json> {
    IndexMap::from([
        (
            String::from("after"),
            query
                .after
                .map_or(Json::Null, |after| Json::Text(after.to_owned())),
        ),
        (
            String::from("before"),
            query
                .before
                .map_or(Json::Null, |before| Json::Text(before.to_owned())),
        ),
        (
            String::from("limit"),
            query
                .limit
                .map_or(Json::Null, |limit| Json::Number(Number::from(limit))),
        ),
        (
            String::from("order"),
            query
                .order
                .map_or(Json::Null, |order| Json::Text(order.to_owned())),
        ),
    ])
}

// the async flag is popped before LiteLLM logs, so it is read and then kept out of the forwarded kwargs
fn log_call<Client>(
    deps: &VectorStoreReadDeps<'_, Client>,
    kwargs: &IndexMap<String, Json>,
    flag: &str,
    optional_params: IndexMap<String, Json>,
    provider: &str,
) -> Result<(), VectorStoreReadError> {
    let mut forwarded = kwargs.clone();
    forwarded.shift_remove(flag);
    let litellm_params = IndexMap::from([(
        String::from("litellm_call_id"),
        forwarded
            .get("litellm_call_id")
            .cloned()
            .unwrap_or(Json::Null),
    )]);
    let additional_params = IndexMap::from([(
        String::from("custom_llm_provider"),
        Json::Text(provider.to_owned()),
    )]);
    let mut state = deps.logging.lock().unwrap_or_else(PoisonError::into_inner);
    update_from_kwargs(
        &mut state,
        &forwarded,
        Some(&litellm_params),
        CallUpdate {
            model: None,
            user: None,
            optional_params,
            additional_params,
        },
    )
    .map_err(VectorStoreReadError::Logging)
}
