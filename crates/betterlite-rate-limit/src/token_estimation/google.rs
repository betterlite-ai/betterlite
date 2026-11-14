use betterlite_models::decisions::Json;

use super::RequestData;
use super::call_family::{CallFamily, call_family};
use super::services::{GenAiTranslation, GenAiTranslationRequest, TranslatedChat};

#[must_use]
pub fn translate_google_request(
    translation: &dyn GenAiTranslation,
    data: &RequestData,
    call_type: Option<&str>,
) -> Option<TranslatedChat> {
    if !matches!(call_family(call_type), CallFamily::Google) {
        return None;
    }
    let contents = data.get("contents")?;
    if !matches!(contents, Json::Object(_) | Json::List(_)) {
        return None;
    }
    let config = object_option(data.get("config").or_else(|| data.get("generationConfig")));
    let model = match data.get("model") {
        Some(Json::Text(model)) => model.as_str(),
        Some(_) | None => "",
    };
    translation.translate_generate_content(&GenAiTranslationRequest {
        model,
        contents,
        config,
        system_instruction: data.get("systemInstruction"),
        system_instruction_snake: data.get("system_instruction"),
        tools: data.get("tools"),
        tool_config: data.get("toolConfig"),
        tool_config_snake: data.get("tool_config"),
    })
}

// the adapter only receives a generation config that is a mapping, and an absent "config" falls back once
const fn object_option(value: Option<&Json>) -> Option<&Json> {
    match value {
        Some(Json::Object(_)) => value,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}
