use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use serde_json::Number;

use super::records::{ExactDecimal, MaxResponseTokens, Setting};
use super::tools::ToolDefinition;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SessionUpdate {
    #[serde(default)]
    pub session: SessionConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SessionConfig {
    #[serde(default)]
    pub max_response_output_tokens: Option<Setting<MaxResponseTokens>>,
    #[serde(default, deserialize_with = "temperature_setting")]
    pub temperature: Option<Setting<ExactDecimal>>,
    #[serde(default)]
    pub voice: Option<Setting<String>>,
    #[serde(default)]
    pub output_audio_format: Option<Setting<String>>,
    #[serde(default)]
    pub input_audio_format: Option<Setting<String>>,
    #[serde(default)]
    pub output_sample_rate_hertz: Option<Setting<i64>>,
    #[serde(default)]
    pub input_sample_rate_hertz: Option<Setting<i64>>,
    #[serde(default)]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(default)]
    pub instructions: Option<Setting<String>>,
}

pub fn temperature_setting<'de, D>(
    deserializer: D,
) -> Result<Option<Setting<ExactDecimal>>, D::Error>
where
    D: Deserializer<'de>,
{
    match Option::<Number>::deserialize(deserializer)? {
        None => Ok(Some(Setting(None))),
        Some(number) => parse_decimal_lexeme(&number.to_string())
            .map(|value| Some(Setting(Some(ExactDecimal(value)))))
            .map_err(D::Error::custom),
    }
}
