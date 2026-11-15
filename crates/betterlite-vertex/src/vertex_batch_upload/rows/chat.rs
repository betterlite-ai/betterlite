use std::error::Error as StdError;

use betterlite_models::decisions::text::decisions_text;
use betterlite_models::decisions::{DecisionsJson, Json as OutputJson};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::message_content::Message;
use indexmap::IndexMap;

use crate::gemini_options::assembly::{OptionsContext, map_options};
use crate::gemini_options::error::{GeminiOptionsError, decode};
use crate::gemini_request::body::{BodyInput, transform};
use crate::gemini_request::error::encode;
use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::vertex_batch_records::rows::VERTEX_AI_PROVIDER;
use crate::vertex_batch_rows::labels::set_custom_id_labels;
use crate::vertex_batch_upload::line::BatchLine;
use crate::vertex_batch_upload::rows::error::RowError;
use crate::vertex_batch_upload::rows::{RowContext, RowMappers};

const BODY_ORDER: [&str; 9] = [
    "contents",
    "system_instruction",
    "tools",
    "toolConfig",
    "safetySettings",
    "generationConfig",
    "cachedContent",
    "serviceTier",
    "labels",
];

// LiteLLM's `body or {}`, then the Responses bridge for a /responses route, then the Gemini body transform
pub async fn rows<Mappers, Fetcher>(
    line: &BatchLine,
    responses: bool,
    context: &RowContext<'_, Mappers, Fetcher>,
) -> Result<Vec<String>, RowError<Mappers::Error>>
where
    Mappers: RowMappers + Sync,
    Fetcher: RemoteMediaFetcher + Sync,
{
    let body = match &line.body {
        None | Some(Json::Null) => Object::new(),
        Some(Json::Object(fields)) => fields.clone(),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(RowError::Field("body", "a JSON object"));
        }
    };
    let chat = if responses {
        context
            .mappers
            .responses_to_chat(VERTEX_AI_PROVIDER, &body)
            .map_err(RowError::Mapper)?
    } else {
        body
    };
    let (mut request, optional) = vertex_request(&chat, context).await?;
    restore_key_order(&mut request, &optional);
    if let Some(custom_id) = &line.custom_id {
        attach_labels::<Mappers::Error>(&mut request, custom_id)?;
    }
    let row = IndexMap::from([("request".to_owned(), OutputJson::Object(request))]);
    let text = decisions_text(&DecisionsJson::Object(row)).map_err(RowError::Dump)?;
    Ok(vec![text])
}

// a request that already carries labels keeps them, and the correlation labels join them
fn attach_labels<Mapped>(
    request: &mut IndexMap<String, OutputJson>,
    custom_id: &str,
) -> Result<(), RowError<Mapped>>
where
    Mapped: StdError + 'static,
{
    let labels = request
        .entry("labels".to_owned())
        .or_insert_with(|| OutputJson::Object(IndexMap::new()));
    let OutputJson::Object(fields) = labels else {
        return Err(RowError::Field("labels", "an object"));
    };
    set_custom_id_labels(fields, custom_id);
    Ok(())
}

async fn vertex_request<Mappers, Fetcher>(
    chat: &Object,
    context: &RowContext<'_, Mappers, Fetcher>,
) -> Result<(IndexMap<String, OutputJson>, Object), RowError<Mappers::Error>>
where
    Mappers: RowMappers + Sync,
    Fetcher: RemoteMediaFetcher + Sync,
{
    let model = match chat.get("model") {
        None => "",
        Some(Json::Text(text)) => text.as_str(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(RowError::Field("model", "a string"));
        }
    };
    let listed: Vec<Object> = match chat.get("messages") {
        None => Vec::new(),
        Some(value) => decode(value, "messages")?,
    };
    let mapped = context
        .mappers
        .developer_to_system(listed)
        .map_err(RowError::Mapper)?;
    let messages: Vec<Message> = encode(&mapped)?;
    let optional = map_chat_options(chat, model, context)?;
    let no_params = Object::new();
    let input = BodyInput {
        provider: VertexCapabilityProvider::VertexAi,
        model,
        messages: &messages,
        optional_params: &optional,
        litellm_params: &no_params,
        cached_content: None,
        modify_params: context.modify_params,
        schema_template: None,
    };
    let transformed = transform(&input, context.capabilities, context.fetcher).await?;
    Ok((encode(&transformed.body)?, optional))
}

// B1321 builds the body through serde_json::Value, which sorts every object's keys, so the order LiteLLM writes is put back
fn restore_key_order(request: &mut IndexMap<String, OutputJson>, optional: &Object) {
    let rank = |name: &String| {
        BODY_ORDER
            .iter()
            .position(|known| known == name)
            .unwrap_or(BODY_ORDER.len())
    };
    request.sort_by(|left, _, right, _| rank(left).cmp(&rank(right)));
    if let Some(OutputJson::List(items)) = request.get_mut("contents") {
        for item in items {
            if let OutputJson::Object(fields) = item {
                fields.sort_by(|left, _, right, _| (left != "role").cmp(&(right != "role")));
            }
        }
    }
    if let Some(OutputJson::Object(config)) = request.get_mut("generationConfig") {
        let position = |name: &String| optional.get_index_of(name).unwrap_or(usize::MAX);
        config.sort_by(|left, _, right, _| position(left).cmp(&position(right)));
    }
}

// LiteLLM maps the whole chat body with drop_params=false, and a key the mapper does not know is ignored
fn map_chat_options<Mappers, Fetcher>(
    chat: &Object,
    model: &str,
    context: &RowContext<'_, Mappers, Fetcher>,
) -> Result<Object, GeminiOptionsError> {
    let mut optional = Object::new();
    let options = OptionsContext {
        model,
        non_default: chat,
        settings: context.options,
        schema: context.schema,
        thinking: context.thinking,
        support: context.support,
    };
    map_options(chat, &mut optional, &options)?;
    Ok(optional)
}
