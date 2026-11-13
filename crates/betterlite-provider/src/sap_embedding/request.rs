use betterlite_openai::embedding::EmbeddingInput as RequestInput;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

use super::error::SapEmbeddingError;
use crate::sap_models::masking::MaskingModuleConfig;
use crate::sap_models::presence::{Field, Given, optional};
use crate::sap_models::validate_masking_module;

const INPUT_TYPES: [&str; 3] = ["text", "document", "query"];

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmbeddingModel {
    pub name: String,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub version: Given<String>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub params: Field<Object>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub timeout: Field<i64>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub max_retries: Field<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmbeddingsModelConfig {
    pub model: EmbeddingModel,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmbeddingsModules {
    pub embeddings: EmbeddingsModelConfig,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub masking: Field<MaskingModuleConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmbeddingConfig {
    pub modules: EmbeddingsModules,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum EmbeddingText {
    Text(String),
    Texts(Vec<String>),
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingInputType {
    Text,
    Document,
    Query,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmbeddingInput {
    pub text: EmbeddingText,
    #[serde(
        rename = "type",
        default,
        with = "optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub type_: Field<EmbeddingInputType>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmbeddingRequest {
    pub config: EmbeddingConfig,
    pub input: EmbeddingInput,
}

fn model_fields(model: &str, optional_params: &Object) -> Result<Object, SapEmbeddingError> {
    let mut fields = Object::from([
        (String::from("name"), Json::Text(model.to_owned())),
        (
            String::from("version"),
            version(optional_params.get("version"))?,
        ),
        (
            String::from("params"),
            parameters(optional_params.get("parameters"))?,
        ),
    ]);
    for name in ["timeout", "max_retries"] {
        if let Some(given) = optional_params
            .get(name)
            .filter(|value| !matches!(value, Json::Null))
        {
            bounds(name, given)?;
            fields.insert(name.to_owned(), given.clone());
        }
    }
    Ok(fields)
}

fn bounds(name: &str, given: &Json) -> Result<(), SapEmbeddingError> {
    let number = match given {
        Json::Number(number) => number.as_i64(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    };
    let (low, high, error) = match name {
        "timeout" => (1, 600, SapEmbeddingError::Timeout),
        _ => (0, 5, SapEmbeddingError::MaxRetries),
    };
    if number.is_some_and(|value| (low..=high).contains(&value)) {
        return Ok(());
    }
    Err(error)
}

pub fn transform_embedding_request(
    model: &str,
    input: &RequestInput,
    optional_params: &Object,
) -> Result<Json, SapEmbeddingError> {
    let mut modules = Object::from([(
        String::from("embeddings"),
        Json::Object(Object::from([(
            String::from("model"),
            Json::Object(model_fields(model, optional_params)?),
        )])),
    )]);
    if let Some(masking) = optional_params
        .get("masking")
        .filter(|value| !matches!(value, Json::Null))
    {
        modules.insert(String::from("masking"), validate_masking_module(masking)?);
    }
    let mut input_fields = Object::from([(String::from("text"), embedding_text(input)?)]);
    if let Some(kind) = input_type(optional_params.get("type"))? {
        input_fields.insert(String::from("type"), kind);
    }
    let body = Json::Object(Object::from([
        (
            String::from("config"),
            Json::Object(Object::from([(
                String::from("modules"),
                Json::Object(modules),
            )])),
        ),
        (String::from("input"), Json::Object(input_fields)),
    ]));
    let text = serde_json::to_string(&body).map_err(SapEmbeddingError::Encode)?;
    let request: EmbeddingRequest =
        serde_json::from_str(&text).map_err(SapEmbeddingError::Request)?;
    let encoded = serde_json::to_string(&request).map_err(SapEmbeddingError::Encode)?;
    serde_json::from_str(&encoded).map_err(SapEmbeddingError::Encode)
}

fn parameters(given: Option<&Json>) -> Result<Json, SapEmbeddingError> {
    match given {
        None | Some(Json::Null) => Ok(Json::Object(Object::new())),
        Some(value @ Json::Object(_)) => Ok(value.clone()),
        Some(_) => Err(SapEmbeddingError::Parameters),
    }
}

fn version(given: Option<&Json>) -> Result<Json, SapEmbeddingError> {
    match given {
        None | Some(Json::Null) => Ok(Json::Text(String::from("latest"))),
        Some(value @ Json::Text(_)) => Ok(value.clone()),
        Some(_) => Err(SapEmbeddingError::Version),
    }
}

fn input_type(given: Option<&Json>) -> Result<Option<Json>, SapEmbeddingError> {
    match given {
        None | Some(Json::Null) => Ok(None),
        Some(value @ Json::Text(kind)) if INPUT_TYPES.contains(&kind.as_str()) => {
            Ok(Some(value.clone()))
        }
        Some(_) => Err(SapEmbeddingError::InputType),
    }
}

fn embedding_text(input: &RequestInput) -> Result<Json, SapEmbeddingError> {
    match input {
        RequestInput::Text(text) => Ok(Json::Text(text.clone())),
        RequestInput::Texts(texts) => {
            Ok(Json::List(texts.iter().cloned().map(Json::Text).collect()))
        }
        RequestInput::Tokens(_) | RequestInput::TokenLists(_) => Err(SapEmbeddingError::Input),
    }
}
