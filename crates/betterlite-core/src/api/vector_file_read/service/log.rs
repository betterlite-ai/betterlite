use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

use super::ReadCall;
use crate::api::vector_file_read::error::VectorFileReadError;

// update_from_kwargs(model=None, optional_params={ids}, litellm_params={vector_store_id, litellm_call_id, **model_dump})
pub(super) fn log_read(
    state: &mut LoggingState,
    call: &ReadCall<'_>,
    kwargs: &JsonObject,
    call_id: &Json,
    provider: &str,
) -> Result<(), VectorFileReadError> {
    let optional_params = IndexMap::from([
        (
            "vector_store_id".to_owned(),
            Json::Text(call.vector_store_id.to_owned()),
        ),
        ("file_id".to_owned(), Json::Text(call.file_id.to_owned())),
    ]);
    let mut litellm_params = IndexMap::from([
        (
            "vector_store_id".to_owned(),
            Json::Text(call.vector_store_id.to_owned()),
        ),
        ("litellm_call_id".to_owned(), call_id.clone()),
    ]);
    litellm_params.extend(generic_params_dump(kwargs));
    let additional_params = IndexMap::from([(
        "custom_llm_provider".to_owned(),
        Json::Text(provider.to_owned()),
    )]);
    update_from_kwargs(
        state,
        kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: None,
            user: None,
            optional_params,
            additional_params,
        },
    )?;
    Ok(())
}

// GenericLiteLLMParams.model_dump(exclude_none=True): the five fields defaulting to False, then every non-null kwarg
fn generic_params_dump(kwargs: &JsonObject) -> IndexMap<String, Json> {
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
