use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
use serde_json::Number;

use super::error::GeminiOptionsError;

// LiteLLM's class attributes start unset, whatever the docstring's example values say
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct GenerationDefaults {
    pub temperature: Option<f64>,
    pub max_output_tokens: Option<i64>,
    pub top_p: Option<f64>,
    pub top_k: Option<i64>,
    pub response_mime_type: Option<String>,
    pub candidate_count: Option<i64>,
    pub stop_sequences: Option<Vec<String>>,
    pub frequency_penalty: Option<f64>,
    pub presence_penalty: Option<f64>,
    pub seed: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct OptionSettings {
    pub generation: GenerationDefaults,
    // LiteLLM reads its module-level vertex_ai_safety_settings; None means the config leaves it unset
    pub safety_settings: Option<Json>,
}

impl GenerationDefaults {
    // get_config() returns only the attributes the config constructed with a value
    pub fn get_config(&self) -> Result<Object, GeminiOptionsError> {
        let mut config = Object::new();
        self.merge_into(&mut config)?;
        Ok(config)
    }

    // a configured value fills only what the request left out
    pub fn merge_into(&self, options: &mut Object) -> Result<(), GeminiOptionsError> {
        insert_float(options, "temperature", self.temperature)?;
        insert_float(options, "top_p", self.top_p)?;
        insert_float(options, "frequency_penalty", self.frequency_penalty)?;
        insert_float(options, "presence_penalty", self.presence_penalty)?;
        insert_integer(options, "max_output_tokens", self.max_output_tokens);
        insert_integer(options, "top_k", self.top_k);
        insert_integer(options, "candidate_count", self.candidate_count);
        insert_integer(options, "seed", self.seed);
        if let Some(mime) = &self.response_mime_type {
            options
                .entry("response_mime_type".to_owned())
                .or_insert(Json::Text(mime.clone()));
        }
        if let Some(items) = &self.stop_sequences {
            let sequences = items.iter().cloned().map(Json::Text).collect();
            options
                .entry("stop_sequences".to_owned())
                .or_insert(Json::List(sequences));
        }
        Ok(())
    }
}

fn insert_float(
    options: &mut Object,
    name: &'static str,
    configured: Option<f64>,
) -> Result<(), GeminiOptionsError> {
    let Some(value) = configured else {
        return Ok(());
    };
    let number = Number::from_f64(value).ok_or_else(|| GeminiOptionsError::NonFinite {
        field: name.to_owned(),
    })?;
    options
        .entry(name.to_owned())
        .or_insert(Json::Number(number));
    Ok(())
}

fn insert_integer(options: &mut Object, name: &'static str, configured: Option<i64>) {
    if let Some(value) = configured {
        options
            .entry(name.to_owned())
            .or_insert(Json::Number(Number::from(value)));
    }
}
