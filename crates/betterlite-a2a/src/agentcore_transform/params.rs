use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use secrecy::SecretString;

use super::TransformError;

#[derive(Clone)]
pub struct LitellmParams {
    pub model: String,
    pub api_key: Option<SecretString>,
    pub principal: Option<String>,
    pub optional: BTreeMap<String, AgentJson>,
}

impl LitellmParams {
    pub fn parse(raw: &BTreeMap<String, AgentJson>) -> Result<Self, TransformError> {
        let model = text(raw.get("model"), "model")?
            .unwrap_or_default()
            .to_owned();
        let api_key = text(raw.get("api_key"), "api_key")?.map(SecretString::from);
        let principal = text(
            raw.get("litellm_a2a_user_api_key_hash"),
            "litellm_a2a_user_api_key_hash",
        )?
        .map(str::to_owned);
        let optional = raw
            .iter()
            .filter(|(name, _)| !matches!(name.as_str(), "model" | "custom_llm_provider"))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        Ok(Self {
            model,
            api_key,
            principal,
            optional,
        })
    }
}

fn text<'value>(
    value: Option<&'value AgentJson>,
    field: &'static str,
) -> Result<Option<&'value str>, TransformError> {
    match value {
        None | Some(AgentJson::Null) => Ok(None),
        Some(AgentJson::String(string)) => Ok(Some(string.as_str())),
        Some(
            AgentJson::Bool(_) | AgentJson::Number(_) | AgentJson::Array(_) | AgentJson::Object(_),
        ) => Err(TransformError::FieldType { field }),
    }
}
