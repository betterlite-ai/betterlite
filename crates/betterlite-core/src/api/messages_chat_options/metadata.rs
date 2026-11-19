use betterlite_prompts::arguments::json::{Json, Object};

use crate::api::messages_chat_options::capabilities::supports_prompt_cache_key;
use crate::api::messages_chat_options::context::OptionsContext;
use crate::api::messages_chat_options::error::OptionsError;
use crate::api::messages_thinking::session::decode_messages_thinking;

// metadata.user_id becomes user verbatim, and only a JSON session id inside it becomes a cache key.
pub fn translate_metadata(
    request: &mut Object,
    kwargs: &mut Object,
    context: &OptionsContext<'_>,
) -> Result<(), OptionsError> {
    if let Some(user_id) = request
        .get("metadata")
        .filter(|metadata| metadata.is_truthy())
        .and_then(Json::as_object)
        .and_then(|fields| fields.get("user_id"))
    {
        kwargs.insert("user".to_owned(), user_id.clone());
        if let Json::Text(text) = user_id {
            let session = decode_messages_thinking(text.as_bytes(), &context.limits)?;
            if let Some(key) = session.filter(|_| supports_prompt_cache_key(context)) {
                kwargs.insert("prompt_cache_key".to_owned(), Json::Text(key));
            }
        }
    }
    if let Some(litellm_metadata) = request.shift_remove("litellm_metadata") {
        kwargs.insert("metadata".to_owned(), litellm_metadata);
    }
    Ok(())
}
