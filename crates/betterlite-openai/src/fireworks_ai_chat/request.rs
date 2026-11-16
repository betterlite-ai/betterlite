use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::fireworks_catalog_metadata::{Catalog, resolve_resource_name};
use betterlite_prompts::arguments::json::{Json, Object};

use crate::chat::ChatMessage;
use crate::chat_content::{ChatContent, Mode, UrlDownloader};
use crate::fireworks_ai_chat::PROVIDER;
use crate::fireworks_ai_chat::error::FireworksChatError;
use crate::fireworks_ai_chat::messages::transform_messages;
use crate::fireworks_ai_chat::tools::transform_tools;

const TOOLS: &str = "tools";
const STREAM: &str = "stream";
const STREAM_OPTIONS: &str = "stream_options";
const INCLUDE_USAGE: &str = "include_usage";

pub struct RequestInput<'request> {
    pub model: &'request str,
    pub messages: &'request [ChatMessage],
    pub optional_params: &'request Object,
    pub catalog: &'request Catalog,
    pub environment: &'request dyn EnvironmentLookup,
    pub downloads: &'request dyn UrlDownloader,
}

// transform_request: resolve the resource name, clean messages and tools, fill stream_options, then B503
pub fn transform_request(input: &RequestInput<'_>) -> Result<Object, FireworksChatError> {
    let model = resolve_resource_name(input.model);
    let messages = transform_messages(input.messages, &model, input.catalog)?;
    let mut optional_params = input.optional_params.clone();
    match optional_params.get_mut(TOOLS) {
        Some(Json::Null) | None => {}
        Some(tools) => transform_tools(tools)?,
    }
    apply_stream_options(&mut optional_params)?;
    let content = ChatContent {
        model: &model,
        messages: &messages,
        optional_params: &optional_params,
        provider: Some(PROVIDER),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment: input.environment,
        downloads: input.downloads,
    };
    let transformed = content.transform(Mode::Sync)?;
    Ok(transformed.to_body()?)
}

// stream_options None becomes include_usage true, a map keeps its entries unless include_usage is false
fn apply_stream_options(optional_params: &mut Object) -> Result<(), FireworksChatError> {
    if !matches!(optional_params.get(STREAM), Some(Json::Bool(true))) {
        return Ok(());
    }
    match optional_params.get(STREAM_OPTIONS).cloned() {
        None | Some(Json::Null) => {
            let options = Object::from([(INCLUDE_USAGE.to_owned(), Json::Bool(true))]);
            optional_params.insert(STREAM_OPTIONS.to_owned(), Json::Object(options));
            Ok(())
        }
        Some(Json::Object(mut options)) => {
            if !matches!(options.get(INCLUDE_USAGE), Some(Json::Bool(false))) {
                options.insert(INCLUDE_USAGE.to_owned(), Json::Bool(true));
            }
            optional_params.insert(STREAM_OPTIONS.to_owned(), Json::Object(options));
            Ok(())
        }
        Some(_) => Err(FireworksChatError::StreamOptions),
    }
}
