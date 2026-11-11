use betterlite_openai::error::ProviderError;

use crate::ollama_chat::error::OllamaError;
use crate::ollama_generate::error::OllamaGenerateError;

#[test]
fn a_chat_error_becomes_a_provider_error_with_its_status() {
    let error = OllamaGenerateError::from(OllamaError::Param { field: "x" });
    assert_eq!(ProviderError::from(error).status_code, 400);
}

#[test]
fn a_message_error_becomes_a_provider_error_with_a_fallback_status() {
    let error = OllamaGenerateError::Message {
        source: serde_json::from_str::<i64>("not json").unwrap_err(),
    };
    assert_eq!(ProviderError::from(error).status_code, 500);
}
