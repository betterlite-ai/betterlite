use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object as PromptObject;
use serde_json::Value;
use tracing::warn;

use crate::chat::ChatMessage;
use crate::chat_content::{ChatContent, Mode, RefuseDownloads};
use crate::huggingface_chat::error::{HuggingfaceChatError, get_error_class};
use crate::huggingface_chat::mapping::ProviderMappingSource;
use crate::openai_base_config::{Headers, Object};

pub fn transform_request(
    model: &str,
    messages: &[ChatMessage],
    optional_params: &Object,
    litellm_api_base: Option<&str>,
    source: &dyn ProviderMappingSource,
) -> Result<Object, HuggingfaceChatError> {
    if litellm_api_base.is_some_and(|base| !base.is_empty()) {
        return request_body(model, messages, optional_params);
    }
    let mut params = optional_params.clone();
    if params.remove("max_retries").is_some() {
        warn!("`max_retries` is not supported. It will be ignored.");
    }
    let sent_model = mapped_model(model, source)?;
    let transformed = transformed_messages(messages, &sent_model)?;
    request_body(&sent_model, &transformed, &params)
}

fn mapped_model(
    model: &str,
    source: &dyn ProviderMappingSource,
) -> Result<String, HuggingfaceChatError> {
    let (first_part, remaining) =
        model
            .split_once('/')
            .ok_or_else(|| HuggingfaceChatError::MissingProviderPrefix {
                model: model.to_owned(),
            })?;
    if !remaining.contains('/') {
        return Ok(model.to_owned());
    }
    let provider = first_part;
    let mapping = source.mapping(remaining)?;
    let Some(entry) = mapping.get(provider) else {
        let message = format!("Model {remaining} is not supported for provider {provider}");
        return Err(HuggingfaceChatError::Huggingface(get_error_class(
            &message,
            404,
            &Headers::new(),
        )));
    };
    if entry.status == "staging" {
        warn!(
            "Model {remaining} is in staging mode for provider {provider}. Meant for test purposes only."
        );
    }
    Ok(entry.provider_id.clone())
}

fn transformed_messages(
    messages: &[ChatMessage],
    model: &str,
) -> Result<Vec<ChatMessage>, HuggingfaceChatError> {
    // the shared message transform reads only the messages and the provider, never the request options
    let options = PromptObject::new();
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let downloads = RefuseDownloads;
    let content = ChatContent {
        model,
        messages,
        optional_params: &options,
        provider: Some("huggingface"),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment: &environment,
        downloads: &downloads,
    };
    Ok(content.transform(Mode::Sync)?.messages)
}

fn request_body(
    model: &str,
    messages: &[ChatMessage],
    optional_params: &Object,
) -> Result<Object, HuggingfaceChatError> {
    let serialized = serde_json::to_value(messages)
        .map_err(|source| HuggingfaceChatError::MessageEncode { source })?;
    let mut body = Object::from_iter([
        ("model".to_owned(), Value::String(model.to_owned())),
        ("messages".to_owned(), serialized),
    ]);
    body.extend(optional_params.clone());
    Ok(body)
}
