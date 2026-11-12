use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::env_integer::get_env_int;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[path = "prompt_injection/tests.rs"]
mod tests;

const MAX_THREADS_VARIABLE: &str = "PROMPT_INJECTION_HEURISTICS_MAX_THREADS";
const SIMILARITY_THRESHOLD_VARIABLE: &str = "DEFAULT_PROMPT_INJECTION_SIMILARITY_THRESHOLD";
const DEFAULT_MAX_THREADS: i64 = 1;
const MINIMUM_MAX_THREADS: usize = 1;
// 0.7 at scale 1, LiteLLM's DEFAULT_PROMPT_INJECTION_SIMILARITY_THRESHOLD
const DEFAULT_SIMILARITY_THRESHOLD: Decimal = Decimal::from_parts(7, 0, 0, false, 1);

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    #[serde(rename = "litellm_settings")]
    settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    prompt_injection_params: Option<PromptInjectionParams>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub params: Option<PromptInjectionParams>,
    pub heuristics_max_threads: usize,
    pub similarity_threshold: Decimal,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            params: None,
            heuristics_max_threads: MINIMUM_MAX_THREADS,
            similarity_threshold: DEFAULT_SIMILARITY_THRESHOLD,
        }
    }
}

// a bool alias, like GuardrailFlag: clippy's two-bool limit counts the primitive spelling only
pub type PromptInjectionFlag = bool;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "RawPromptInjectionParams")]
pub struct PromptInjectionParams {
    pub heuristics_check: PromptInjectionFlag,
    pub vector_db_check: PromptInjectionFlag,
    pub llm_api_check: PromptInjectionFlag,
    pub llm_api_name: Option<String>,
    pub llm_api_system_prompt: Option<String>,
    pub llm_api_fail_call_string: Option<String>,
    pub reject_as_response: Option<bool>,
}

impl Default for PromptInjectionParams {
    fn default() -> Self {
        Self {
            heuristics_check: false,
            vector_db_check: false,
            llm_api_check: false,
            llm_api_name: None,
            llm_api_system_prompt: None,
            llm_api_fail_call_string: None,
            reject_as_response: Some(false),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawPromptInjectionParams {
    #[serde(default)]
    heuristics_check: PromptInjectionFlag,
    #[serde(default)]
    vector_db_check: PromptInjectionFlag,
    #[serde(default)]
    llm_api_check: PromptInjectionFlag,
    #[serde(default)]
    llm_api_name: Option<String>,
    #[serde(default)]
    llm_api_system_prompt: Option<String>,
    #[serde(default)]
    llm_api_fail_call_string: Option<String>,
    #[serde(default = "default_reject_as_response")]
    reject_as_response: Option<bool>,
    // pydantic's default config ignores keys the model does not declare; Option keeps the sink non-zero-sized
    #[serde(flatten)]
    _unknown: BTreeMap<String, Option<IgnoredAny>>,
}

pub const fn default_reject_as_response() -> Option<bool> {
    Some(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PromptInjectionParamsError {
    #[error("If llm_api_check is set to True, llm_api_name must be provided")]
    LlmApiNameMissing,
    #[error("If llm_api_check is set to True, llm_api_system_prompt must be provided")]
    LlmApiSystemPromptMissing,
    #[error("If llm_api_check is set to True, llm_api_fail_call_string must be provided")]
    LlmApiFailCallStringMissing,
}

impl TryFrom<RawPromptInjectionParams> for PromptInjectionParams {
    type Error = PromptInjectionParamsError;

    fn try_from(raw: RawPromptInjectionParams) -> Result<Self, Self::Error> {
        let params = Self {
            heuristics_check: raw.heuristics_check,
            vector_db_check: raw.vector_db_check,
            llm_api_check: raw.llm_api_check,
            llm_api_name: raw.llm_api_name,
            llm_api_system_prompt: raw.llm_api_system_prompt,
            llm_api_fail_call_string: raw.llm_api_fail_call_string,
            reject_as_response: raw.reject_as_response,
        };
        if !params.llm_api_check {
            return Ok(params);
        }
        if params.llm_api_name.as_deref().is_none_or(str::is_empty) {
            return Err(PromptInjectionParamsError::LlmApiNameMissing);
        }
        if params
            .llm_api_system_prompt
            .as_deref()
            .is_none_or(str::is_empty)
        {
            return Err(PromptInjectionParamsError::LlmApiSystemPromptMissing);
        }
        if params
            .llm_api_fail_call_string
            .as_deref()
            .is_none_or(str::is_empty)
        {
            return Err(PromptInjectionParamsError::LlmApiFailCallStringMissing);
        }
        Ok(params)
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            params: self.settings.prompt_injection_params,
            heuristics_max_threads: heuristics_max_threads(lookup),
            similarity_threshold: similarity_threshold(lookup)?,
        })
    }
}

// LiteLLM takes max(1, get_env_int(...)), so zero and negatives become one worker
fn heuristics_max_threads<L>(lookup: &L) -> usize
where
    L: EnvironmentLookup,
{
    let parsed = get_env_int(
        lookup.lookup(MAX_THREADS_VARIABLE).as_deref(),
        DEFAULT_MAX_THREADS,
    );
    usize::try_from(parsed)
        .unwrap_or(MINIMUM_MAX_THREADS)
        .max(MINIMUM_MAX_THREADS)
}

// LiteLLM calls float() here, so a value that is not a number stops startup and names the variable
fn similarity_threshold<L>(lookup: &L) -> Result<Decimal, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(SIMILARITY_THRESHOLD_VARIABLE) else {
        return Ok(DEFAULT_SIMILARITY_THRESHOLD);
    };
    Ok(
        betterlite_env::number::<Decimal>(SIMILARITY_THRESHOLD_VARIABLE, &text)?
            .unwrap_or(DEFAULT_SIMILARITY_THRESHOLD),
    )
}
