use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::vector_search_execution::config::Params;
use crate::api::vector_store_search::error::VectorStoreSearchError;

pub(super) fn log_update(
    state: &mut LoggingState,
    kwargs: &Params,
    litellm_params: &IndexMap<String, Json>,
    update: CallUpdate,
) -> Result<(), VectorStoreSearchError> {
    update_from_kwargs(state, &as_index_map(kwargs), Some(litellm_params), update)?;
    Ok(())
}

fn as_index_map(params: &Params) -> IndexMap<String, Json> {
    params
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

// GenericLiteLLMParams.model_dump(exclude_none=True): the five fields defaulting to False, then every non-null kwarg
pub(super) fn generic_params_dump(kwargs: &Params) -> IndexMap<String, Json> {
    let mut dump: IndexMap<String, Json> = [
        "allow_client_keepalive_override",
        "merge_reasoning_content_in_choices",
        "use_in_pass_through",
        "use_litellm_proxy",
        "use_xai_oauth",
    ]
    .into_iter()
    .map(|name| (name.to_owned(), Json::Bool(false)))
    .collect();
    dump.extend(
        kwargs
            .iter()
            .filter(|(_, value)| !matches!(value, Json::Null))
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    dump
}
