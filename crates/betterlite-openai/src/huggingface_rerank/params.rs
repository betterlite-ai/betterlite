use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

use crate::huggingface_rerank::error::HuggingfaceRerankError;

pub type Params = BTreeMap<String, Json>;

pub const SUPPORTED_RERANK_PARAMS: [&str; 4] = ["query", "documents", "top_n", "return_documents"];

#[must_use]
pub fn map_cohere_rerank_params(non_default_params: Option<&Params>) -> Params {
    let mut mapped = Params::new();
    for (name, value) in non_default_params.into_iter().flatten() {
        if *value == Json::Null {
            continue;
        }
        let mapped_name = match name.as_str() {
            "documents" => "texts",
            "return_documents" if matches!(value, Json::Bool(_)) => "return_text",
            "return_documents" => continue,
            "top_n" | "query" => name.as_str(),
            _ => continue,
        };
        mapped.insert(mapped_name.to_owned(), value.clone());
    }
    mapped
}

pub fn transform_rerank_request(
    optional_rerank_params: &Params,
) -> Result<Params, HuggingfaceRerankError> {
    if !optional_rerank_params.contains_key("query") {
        return Err(HuggingfaceRerankError::MissingQuery);
    }
    if !optional_rerank_params.contains_key("texts") {
        return Err(HuggingfaceRerankError::MissingTexts);
    }
    let mut body = Params::from([
        ("raw_scores".to_owned(), Json::Bool(false)),
        ("truncate".to_owned(), Json::Bool(false)),
        (
            "truncation_direction".to_owned(),
            Json::Text("Right".to_owned()),
        ),
    ]);
    body.extend(
        optional_rerank_params
            .iter()
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    Ok(body)
}
