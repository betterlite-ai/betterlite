use std::collections::BTreeMap;

use super::presence::Field;
use super::presence::optional;
use betterlite_prompts::arguments::json::Json;
use serde::{Deserialize, Serialize};

use super::SapModelError;
use super::filtering::FilteringModuleConfig;
use super::grounding::GroundingModuleConfig;
use super::masking::MaskingModuleConfig;
use super::parse_nested;
use super::presence::Given;
use super::template::PromptTemplatingModuleConfig;
use super::translation::TranslationModuleConfig;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModuleConfig {
    pub prompt_templating: PromptTemplatingModuleConfig,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub filtering: Field<FilteringModuleConfig>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub masking: Field<MaskingModuleConfig>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub grounding: Field<GroundingModuleConfig>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub translation: Field<TranslationModuleConfig>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(try_from = "i64")]
pub struct ChunkSize(i64);

impl TryFrom<i64> for ChunkSize {
    type Error = SapModelError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if value < 1 {
            return Err(SapModelError::Rejected {
                message: String::from("chunk_size must be at least 1"),
            });
        }
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GlobalStreamOptions {
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub enabled: Given<bool>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub chunk_size: Field<ChunkSize>,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub delimiters: Field<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged, try_from = "Json")]
pub enum Modules {
    One(Box<ModuleConfig>),
    Many(Vec<ModuleConfig>),
}

impl TryFrom<Json> for Modules {
    type Error = SapModelError;

    fn try_from(value: Json) -> Result<Self, Self::Error> {
        match &value {
            Json::Object(_) => Ok(Self::One(Box::new(parse_nested(&value)?))),
            Json::List(_) => Ok(Self::Many(parse_nested(&value)?)),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {
                Err(SapModelError::Rejected {
                    message: String::from("modules must be an object or a list of objects"),
                })
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OrchestrationConfig {
    pub modules: Modules,
    #[serde(default, with = "optional", skip_serializing_if = "Option::is_none")]
    pub stream: Field<GlobalStreamOptions>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OrchestrationRequest {
    pub config: OrchestrationConfig,
    #[serde(
        rename = "placeholder_values",
        default,
        with = "optional",
        skip_serializing_if = "Option::is_none"
    )]
    pub placeholder_values: Field<BTreeMap<String, String>>,
}
