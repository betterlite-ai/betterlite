use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::VideoContentCallError;

// update_from_kwargs(kwargs, model="", user=..., optional_params={"video_id": ...}, litellm_params={"litellm_call_id": ..., "video_id": ...}, custom_llm_provider=...)
pub(crate) fn log_call(
    logging: &mut LoggingState,
    kwargs: &IndexMap<String, Json>,
    video_id: &str,
    provider: &str,
) -> Result<(), VideoContentCallError> {
    let mut optional_params = IndexMap::new();
    optional_params.insert("video_id".to_owned(), Json::Text(video_id.to_owned()));
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        kwargs.get("litellm_call_id").cloned().unwrap_or(Json::Null),
    );
    litellm_params.insert("video_id".to_owned(), Json::Text(video_id.to_owned()));
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(provider.to_owned()),
    );
    let user = kwargs_user(kwargs)?;
    update_from_kwargs(
        logging,
        kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: Some(String::new()),
            user,
            optional_params,
            additional_params,
        },
    )?;
    Ok(())
}

// kwargs.get("user") as LiteLLM hands it over, with any other type refused by docs/compatibility.md
fn kwargs_user(kwargs: &IndexMap<String, Json>) -> Result<Option<String>, VideoContentCallError> {
    match kwargs.get("user") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(user)) => Ok(Some(user.clone())),
        Some(_) => Err(VideoContentCallError::FieldType {
            field: "user",
            expected: "a string",
        }),
    }
}
