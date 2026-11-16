use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingInput;
use serde_json::Number;

fn tokens_json(tokens: &[u32]) -> Json {
    Json::List(
        tokens
            .iter()
            .map(|token| Json::Number(Number::from(*token)))
            .collect(),
    )
}

#[must_use]
pub fn input_json(input: &EmbeddingInput) -> Json {
    match input {
        EmbeddingInput::Text(text) => Json::Text(text.clone()),
        EmbeddingInput::Texts(texts) => {
            Json::List(texts.iter().map(|text| Json::Text(text.clone())).collect())
        }
        EmbeddingInput::Tokens(tokens) => tokens_json(tokens),
        EmbeddingInput::TokenLists(lists) => {
            Json::List(lists.iter().map(|tokens| tokens_json(tokens)).collect())
        }
    }
}
