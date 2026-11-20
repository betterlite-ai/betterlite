use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json as prompts;

use crate::api::messages_chat_transform::compose::translate_anthropic_to_openai;
use crate::api::messages_chat_transform::settings::AdapterSettings;
use crate::api::messages_content_primitives::tools::TRANSLATABLE_ANTHROPIC_PARAMS;
use crate::shadow_requests::Object;
use crate::shadow_requests::error::ShadowRequestError;

use super::environment::{NoCapabilities, NoParameters};
use super::{ChatRequestFuture, model, or_value, proxy_wire_body};

const MESSAGES: &str = "messages";
const MODEL: &str = "model";
const SYSTEM: &str = "system";

const SAMPLING_PARAMS: [&str; 5] = [
    "max_tokens",
    "temperature",
    "top_p",
    "top_k",
    "reasoning_effort",
];

#[must_use]
pub fn chat_request<'request>(
    kwargs: &'request Object,
    _model_parameters: &'request Object,
) -> ChatRequestFuture<'request> {
    Box::pin(async move {
        let wire_body = proxy_wire_body(kwargs);
        let mut request = Object::new();
        for (name, value) in &wire_body {
            if copied_param(name) {
                request.insert(name.clone(), value.clone());
            }
        }
        request.insert(MODEL.to_owned(), Json::Text(model(kwargs)?));
        request.insert(MESSAGES.to_owned(), Json::List(messages(kwargs)?));
        let system = or_value(kwargs.get(SYSTEM), wire_body.get(SYSTEM))?
            .filter(|value| !matches!(value, Json::Null))
            .cloned();
        if let Some(system) = system {
            request.insert(SYSTEM.to_owned(), system);
        }
        let encoded = serde_json::to_string(&request)?;
        let mut translated_input: prompts::Object = serde_json::from_str(&encoded)?;
        let parameters = NoParameters;
        let settings = AdapterSettings::new(&parameters);
        let mut capabilities = NoCapabilities;
        let (translated, _tool_name_mapping) =
            translate_anthropic_to_openai(&mut translated_input, &settings, &mut capabilities)
                .await?;
        let translated_text = serde_json::to_string(&translated)?;
        Ok(serde_json::from_str(&translated_text)?)
    })
}

fn copied_param(name: &str) -> bool {
    name != MESSAGES
        && name != SYSTEM
        && (TRANSLATABLE_ANTHROPIC_PARAMS.contains(&name) || SAMPLING_PARAMS.contains(&name))
}

fn messages(kwargs: &Object) -> Result<Vec<Json>, ShadowRequestError> {
    match kwargs.get(MESSAGES) {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::List(items)) => items
            .iter()
            .map(|item| match item {
                Json::Object(_) => Ok(item.clone()),
                Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Null => {
                    Err(ShadowRequestError::MessagesNotObjects)
                }
            })
            .collect(),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(ShadowRequestError::MessagesNotObjects)
        }
    }
}
