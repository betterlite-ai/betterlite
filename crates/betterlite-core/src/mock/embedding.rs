use betterlite_config::sections::mock_response::MockResponse;
use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_openai::embedding::{
    EmbeddingRecord, EmbeddingResponse, EmbeddingUsage, EmbeddingVector,
};
use serde_json::Number;

// LiteLLM's mock_embedding fills a 1536-dimension zero vector when it gets no response
pub const DEFAULT_DIMENSIONS: usize = 1536;
pub const ERROR_SENTINEL: &str = "error";
const PROMPT_TOKENS: u32 = 10;

pub fn response(
    response: Option<&MockResponse>,
    model: &str,
) -> Result<EmbeddingResponse, MockError> {
    let embedding = match response {
        None => EmbeddingVector::Floats(vec![
            Number::from_f64(0.0).ok_or(MockError::WrongType {
                field: "mock_response",
            })?;
            DEFAULT_DIMENSIONS
        ]),
        Some(MockResponse::Numbers(values)) => EmbeddingVector::Floats(values.clone()),
        Some(MockResponse::Text(text)) if text == ERROR_SENTINEL => {
            return Err(MockError::EmbeddingFailure);
        }
        Some(MockResponse::Text(_) | MockResponse::Object(_)) => {
            return Err(MockError::WrongType {
                field: "mock_response",
            });
        }
    };
    Ok(EmbeddingResponse {
        object: String::from("list"),
        data: vec![EmbeddingRecord {
            object: String::from("embedding"),
            embedding,
            index: 0,
        }],
        model: model.to_owned(),
        usage: EmbeddingUsage {
            prompt_tokens: PROMPT_TOKENS,
            completion_tokens: 0,
            completion_tokens_details: None,
            prompt_tokens_details: None,
            total_tokens: 0,
        },
    })
}

mod tests;
