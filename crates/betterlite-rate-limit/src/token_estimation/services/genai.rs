use betterlite_core::api::genai_chat_input::input::GenerateContentRequest;
use betterlite_core::api::genai_chat_input::{TranslationContext, translate_generate_content};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{GenAiTranslation, GenAiTranslationRequest, TranslatedChat};

impl GenAiTranslation for TranslationContext<'_> {
    fn translate_generate_content(
        &self,
        request: &GenAiTranslationRequest<'_>,
    ) -> Option<TranslatedChat> {
        let mut body = IndexMap::new();
        body.insert("model".to_owned(), Json::Text(request.model.to_owned()));
        body.insert("contents".to_owned(), request.contents.clone());
        if let Some(config) = request.config {
            body.insert("generationConfig".to_owned(), config.clone());
        }
        if let Some(instruction) = request.system_instruction {
            body.insert("systemInstruction".to_owned(), instruction.clone());
        }
        if let Some(instruction) = request.system_instruction_snake {
            body.insert("system_instruction".to_owned(), instruction.clone());
        }
        if let Some(tools) = request.tools {
            body.insert("tools".to_owned(), tools.clone());
        }
        if let Some(config) = request.tool_config {
            body.insert("toolConfig".to_owned(), config.clone());
        }
        if let Some(config) = request.tool_config_snake {
            body.insert("tool_config".to_owned(), config.clone());
        }
        let value = serde_json::to_value(Json::Object(body)).ok()?;
        let parsed: GenerateContentRequest = serde_json::from_value(value).ok()?;
        let (completion, _) = translate_generate_content(&parsed, None, self).ok()?;
        let messages = serde_json::to_value(&completion.messages).ok()?;
        let tools = match completion.tools.as_ref() {
            Some(tools) => serde_json::to_value(tools)
                .ok()
                .and_then(|value| serde_json::from_value(value).ok()),
            None => None,
        };
        let tool_choice = match completion.tool_choice.as_ref() {
            Some(choice) => serde_json::to_value(choice)
                .ok()
                .and_then(|value| serde_json::from_value(value).ok()),
            None => None,
        };
        Some(TranslatedChat {
            messages: serde_json::from_value(messages).ok()?,
            tools,
            tool_choice,
        })
    }
}
