use betterlite_openai::voyage_embedding::{
    VoyageContextConfig, VoyageEmbeddingConfig, VoyageEmbeddingVariant, endpoint, request,
};
use serde_json::Value;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let text = input
            .get("text")
            .ok_or_else(|| helper_error("text is required"))?;
        let contextual = input
            .get("contextual")
            .ok_or_else(|| helper_error("contextual is required"))?;
        let text_model = text
            .get("model")
            .and_then(Value::as_str)
            .ok_or_else(|| helper_error("text.model is required"))?;
        let text_input = text
            .get("input")
            .cloned()
            .ok_or_else(|| helper_error("text.input is required"))?;
        let text_options = text
            .get("options")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let text_base = text.get("base").and_then(Value::as_str).map(str::to_owned);
        let contextual_model = contextual
            .get("model")
            .and_then(Value::as_str)
            .ok_or_else(|| helper_error("contextual.model is required"))?;
        let contextual_input = contextual
            .get("input")
            .cloned()
            .ok_or_else(|| helper_error("contextual.input is required"))?;
        let contextual_options = contextual
            .get("options")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let contextual_base = contextual
            .get("base")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let text_config = VoyageEmbeddingConfig {
            api_base: text_base,
        };
        let contextual_config = VoyageContextConfig {
            api_base: contextual_base,
        };
        let text_result = serde_json::json!({
            "url": endpoint(text_config.api_base.as_deref(), VoyageEmbeddingVariant::Text),
            "request": request(text_model, text_input, &text_options, VoyageEmbeddingVariant::Text),
        });
        let contextual_result = serde_json::json!({
            "url": endpoint(contextual_config.api_base.as_deref(), VoyageEmbeddingVariant::Contextual),
            "request": request(contextual_model, contextual_input, &contextual_options, VoyageEmbeddingVariant::Contextual),
            "is_contextual": contextual_model.to_lowercase().contains("context"),
        });
        Ok(serde_json::json!({"text": text_result, "contextual": contextual_result}))
    }
}

fn helper_error(message: &str) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("ValidationError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

#[path = "voyage_contextual/tests.rs"]
mod tests;
