use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::convert_content_list_to_str;
use indexmap::IndexMap;
use uuid::Uuid;

use super::error::AgentCoreRequestError;
use super::identifiers::{SESSION_HEADER, USER_HEADER, runtime_session_id, runtime_user_id};
use super::message::{AgentCoreContent, AgentCoreMessage, as_prompt_message, forwarded_block};
use super::multimodal::{forward_multimodal_content, has_non_text_block};
use crate::model_routes::ParamMap;
use crate::signing::headers::set_exact;

pub fn transform_request(
    messages: &[AgentCoreMessage],
    optional_params: &ParamMap,
    litellm_params: &ParamMap,
    headers: &mut Vec<(String, Option<String>)>,
    generated_session_id: Uuid,
) -> Result<IndexMap<String, Json>, AgentCoreRequestError> {
    let last = messages
        .last()
        .ok_or(AgentCoreRequestError::EmptyMessages)?;
    let prompt = convert_content_list_to_str(&as_prompt_message(last));
    let mut payload = IndexMap::from([("prompt".to_owned(), Json::Text(prompt))]);
    if forward_multimodal_content(optional_params, litellm_params)
        && let Some(content) = &last.content
        && has_non_text_block(content)
        && let AgentCoreContent::Parts(parts) = content
    {
        payload.insert(
            "content".to_owned(),
            Json::List(parts.iter().map(forwarded_block).collect()),
        );
    }
    set_exact(
        headers,
        SESSION_HEADER,
        runtime_session_id(optional_params, generated_session_id)?,
    );
    let user_id = runtime_user_id(optional_params)?;
    if let Some(present) = user_id.filter(|value| !value.is_empty()) {
        set_exact(headers, USER_HEADER, present);
    }
    Ok(payload)
}
