use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
use serde_json::Number;

use crate::studio_chat::error::StudioChatError;

// LiteLLM's class attributes start unset, whatever the docstring's example values say
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct StudioSettings {
    pub temperature: Option<f64>,
    pub max_output_tokens: Option<i64>,
    pub top_p: Option<f64>,
    pub top_k: Option<i64>,
    pub response_mime_type: Option<String>,
    pub response_schema: Option<Object>,
    pub candidate_count: Option<i64>,
    pub stop_sequences: Option<Vec<String>>,
}

impl StudioSettings {
    // get_config() returns only the attributes the config constructed with a value
    pub fn get_config(&self) -> Result<Object, StudioChatError> {
        let mut config = Object::new();
        if let Some(temperature) = self.temperature {
            config.insert("temperature".to_owned(), float(temperature, "temperature")?);
        }
        if let Some(top_p) = self.top_p {
            config.insert("top_p".to_owned(), float(top_p, "top_p")?);
        }
        if let Some(value) = self.max_output_tokens {
            config.insert(
                "max_output_tokens".to_owned(),
                Json::Number(Number::from(value)),
            );
        }
        if let Some(value) = self.top_k {
            config.insert("top_k".to_owned(), Json::Number(Number::from(value)));
        }
        if let Some(mime) = &self.response_mime_type {
            config.insert("response_mime_type".to_owned(), Json::Text(mime.clone()));
        }
        if let Some(schema) = &self.response_schema {
            config.insert("response_schema".to_owned(), Json::Object(schema.clone()));
        }
        if let Some(value) = self.candidate_count {
            config.insert(
                "candidate_count".to_owned(),
                Json::Number(Number::from(value)),
            );
        }
        if let Some(sequences) = &self.stop_sequences {
            config.insert(
                "stop_sequences".to_owned(),
                Json::List(sequences.iter().cloned().map(Json::Text).collect()),
            );
        }
        Ok(config)
    }
}

fn float(value: f64, field: &'static str) -> Result<Json, StudioChatError> {
    let number = Number::from_f64(value).ok_or_else(|| StudioChatError::NonFinite {
        field: field.to_owned(),
    })?;
    Ok(Json::Number(number))
}
