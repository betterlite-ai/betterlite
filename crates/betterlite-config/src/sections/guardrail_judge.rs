use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};

const GUARDRAIL_KIND: &str = "llm_as_a_judge";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    guardrails: Vec<Entry>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Entry {
    guardrail_name: Option<String>,
    litellm_params: Parameters,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Parameters {
    guardrail: String,
    judge_model: Option<String>,
    criteria: Option<Vec<Criterion>>,
    overall_threshold: Option<RawNumber>,
    on_failure: Option<String>,
    mode: Option<EventHookParameter>,
    default_on: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub guardrails: Vec<Configured>,
}

#[derive(Debug, Clone)]
pub struct Configured {
    pub guardrail_name: Option<String>,
    pub params: JudgeParams,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct JudgeParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judge_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub criteria: Option<Vec<Criterion>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overall_threshold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_failure: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<EventHookParameter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_on: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Criterion {
    pub name: String,
    pub description: String,
    pub weight: CriterionWeight,
}

// LiteLLM renders a YAML integer weight as "60" and a float one as "60.0" in the judge prompt.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged, try_from = "serde_json::Number")]
pub enum CriterionWeight {
    Integer(i64),
    Float(f64),
}

// serde_json's arbitrary_precision feature visits a float as the map behind serde_json::Number.
impl TryFrom<serde_json::Number> for CriterionWeight {
    type Error = &'static str;

    fn try_from(number: serde_json::Number) -> Result<Self, Self::Error> {
        number
            .as_i64()
            .map(Self::Integer)
            .or_else(|| number.as_f64().map(Self::Float))
            .ok_or("the criterion weight is not a finite number")
    }
}

impl Default for CriterionWeight {
    fn default() -> Self {
        Self::Integer(0)
    }
}

impl CriterionWeight {
    #[must_use]
    pub fn to_decimal(self) -> Option<Decimal> {
        match self {
            Self::Integer(value) => Some(Decimal::from(value)),
            Self::Float(value) => Decimal::try_from(value).ok(),
        }
    }

    // Python renders a float weight as "60.0" where Rust's Display gives "60".
    #[must_use]
    pub fn to_prompt_text(self) -> String {
        match self {
            Self::Integer(value) => value.to_string(),
            Self::Float(value) => {
                let text = value.to_string();
                if text.contains(['.', 'e', 'E']) {
                    text
                } else {
                    format!("{text}.0")
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum EventHookParameter {
    Single(String),
    Many(Vec<String>),
    Mode(GuardrailMode),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GuardrailMode {
    pub tags: BTreeMap<String, TagValue>,
    #[serde(default)]
    pub default: Option<TagValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum TagValue {
    One(String),
    Many(Vec<String>),
}

impl TagValue {
    fn names(&self) -> Vec<&str> {
        match self {
            Self::One(name) => vec![name.as_str()],
            Self::Many(names) => names.iter().map(String::as_str).collect(),
        }
    }
}

impl GuardrailMode {
    #[must_use]
    pub fn hook_names(&self) -> Vec<&str> {
        let mut names = self
            .tags
            .values()
            .flat_map(TagValue::names)
            .collect::<Vec<&str>>();
        if let Some(default) = &self.default {
            names.extend(default.names());
        }
        names
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut guardrails = Vec::new();
        for (index, entry) in self.guardrails.into_iter().enumerate() {
            if entry.litellm_params.guardrail == GUARDRAIL_KIND {
                guardrails.push(entry.resolve(index, lookup)?);
            }
        }
        Ok(Settings { guardrails })
    }
}

impl Entry {
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<Configured, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix = format!("guardrails.{index}.litellm_params");
        let parameters = self.litellm_params;
        let overall_threshold = resolve_number(
            parameters.overall_threshold,
            &format!("{prefix}.overall_threshold"),
            lookup,
            parse_float,
        )?;
        Ok(Configured {
            guardrail_name: self.guardrail_name,
            params: JudgeParams {
                judge_model: parameters.judge_model,
                criteria: parameters.criteria,
                overall_threshold,
                on_failure: parameters.on_failure,
                mode: parameters.mode,
                default_on: parameters.default_on,
            },
        })
    }
}

mod tests;
