use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use super::masking::{mask_api_base, masked_headers};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum RawBody {
    Text(String),
    Object(IndexMap<String, Json>),
}

impl Default for RawBody {
    fn default() -> Self {
        Self::Object(IndexMap::new())
    }
}

impl From<&RawBody> for Json {
    fn from(body: &RawBody) -> Self {
        match body {
            RawBody::Text(text) => Self::Text(text.clone()),
            RawBody::Object(fields) => Self::Object(fields.clone()),
        }
    }
}

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct AdditionalArgs {
    pub api_base: Option<String>,
    pub headers: Option<IndexMap<String, Json>>,
    pub complete_input_dict: Option<RawBody>,
    pub request_str: Option<String>,
    pub openai_sdk: Option<bool>,
    #[serde(flatten)]
    pub passthrough: IndexMap<String, Json>,
}

impl AdditionalArgs {
    // LiteLLM stores the live headers, so a credential reaches every callback; the stored copy carries the masked ones
    #[must_use]
    pub fn recorded(&self) -> Json {
        let mut fields = IndexMap::new();
        if let Some(api_base) = &self.api_base {
            fields.insert("api_base".to_owned(), Json::Text(mask_api_base(api_base)));
        }
        if let Some(headers) = &self.headers {
            fields.insert(
                "headers".to_owned(),
                Json::Object(masked_headers(headers, false)),
            );
        }
        if let Some(body) = &self.complete_input_dict {
            fields.insert("complete_input_dict".to_owned(), Json::from(body));
        }
        if let Some(request_str) = &self.request_str {
            fields.insert("request_str".to_owned(), Json::Text(request_str.clone()));
        }
        if let Some(openai_sdk) = self.openai_sdk {
            fields.insert("openai_sdk".to_owned(), Json::Bool(openai_sdk));
        }
        fields.extend(self.passthrough.clone());
        Json::Object(fields)
    }
}
