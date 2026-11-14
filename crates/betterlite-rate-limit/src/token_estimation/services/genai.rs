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
        let optional = [
            ("generationConfig", request.config),
            ("systemInstruction", request.system_instruction),
            ("system_instruction", request.system_instruction_snake),
            ("tools", request.tools),
            ("toolConfig", request.tool_config),
            ("tool_config", request.tool_config_snake),
        ];
        for (key, entry) in optional {
            if let Some(field) = entry {
                body.insert(key.to_owned(), field.clone());
            }
        }
        let value = serde_json::to_value(Json::Object(body)).ok()?;
        let parsed: GenerateContentRequest = serde_json::from_value(value).ok()?;
        let (completion, _) = translate_generate_content(&parsed, None, self).ok()?;
        let messages = serde_json::to_value(&completion.messages).ok()?;
        let tools = completion
            .tools
            .as_ref()
            .and_then(|tools| serde_json::from_value(serde_json::to_value(tools).ok()?).ok());
        let tool_choice = completion
            .tool_choice
            .as_ref()
            .and_then(|choice| serde_json::from_value(serde_json::to_value(choice).ok()?).ok());
        Some(TranslatedChat {
            messages: serde_json::from_value(messages).ok()?,
            tools,
            tool_choice,
        })
    }
}
