use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_judge::{EventHookParameter, GuardrailMode};
use serde::{Deserialize, Serialize};

use crate::generic_guardrail_contracts::GuardrailInputType;

use super::error::JudgeError;

pub const INTERNAL_CALL_ORIGIN: &str = "internal_call_origin";
pub const JUDGE_CALL_ORIGIN: &str = "llm_as_a_judge_guardrail";

pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHook; 3] = [
    GuardrailEventHook::PreCall,
    GuardrailEventHook::DuringCall,
    GuardrailEventHook::PostCall,
];

const REQUEST_LIFECYCLE: [GuardrailEventHook; 3] = [
    GuardrailEventHook::PreCall,
    GuardrailEventHook::DuringCall,
    GuardrailEventHook::LoggingOnly,
];

const RESPONSE_LIFECYCLE: [GuardrailEventHook; 2] = [
    GuardrailEventHook::PostCall,
    GuardrailEventHook::LoggingOnly,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailEventHook {
    PreCall,
    PostCall,
    DuringCall,
    LoggingOnly,
    PreMcpCall,
    DuringMcpCall,
    PostMcpCall,
    RealtimeInputTranscription,
}

impl GuardrailEventHook {
    pub fn parse(name: &str) -> Result<Self, JudgeError> {
        match name {
            "pre_call" => Ok(Self::PreCall),
            "post_call" => Ok(Self::PostCall),
            "during_call" => Ok(Self::DuringCall),
            "logging_only" => Ok(Self::LoggingOnly),
            "pre_mcp_call" => Ok(Self::PreMcpCall),
            "during_mcp_call" => Ok(Self::DuringMcpCall),
            "post_mcp_call" => Ok(Self::PostMcpCall),
            "realtime_input_transcription" => Ok(Self::RealtimeInputTranscription),
            _ => Err(JudgeError::InvalidEventHook {
                value: name.to_owned(),
            }),
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PreCall => "pre_call",
            Self::PostCall => "post_call",
            Self::DuringCall => "during_call",
            Self::LoggingOnly => "logging_only",
            Self::PreMcpCall => "pre_mcp_call",
            Self::DuringMcpCall => "during_mcp_call",
            Self::PostMcpCall => "post_mcp_call",
            Self::RealtimeInputTranscription => "realtime_input_transcription",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EventHookConfig {
    Single(GuardrailEventHook),
    Many(Vec<GuardrailEventHook>),
    Mode(GuardrailMode),
}

impl EventHookConfig {
    #[must_use]
    pub fn is_configured(&self, event_type: GuardrailEventHook) -> bool {
        match self {
            Self::Single(configured) => *configured == event_type,
            Self::Many(configured) => configured.contains(&event_type),
            Self::Mode(mode) => mode
                .hook_names()
                .into_iter()
                .any(|name| name == event_type.as_str()),
        }
    }

    fn configured_hooks(&self) -> Result<Vec<GuardrailEventHook>, JudgeError> {
        match self {
            Self::Single(hook) => Ok(vec![*hook]),
            Self::Many(hooks) => Ok(hooks.clone()),
            Self::Mode(mode) => mode
                .hook_names()
                .into_iter()
                .map(GuardrailEventHook::parse)
                .collect(),
        }
    }
}

pub fn coerce_event_hook(
    parameter: Option<&EventHookParameter>,
) -> Result<EventHookConfig, JudgeError> {
    match parameter {
        None => Ok(EventHookConfig::Single(GuardrailEventHook::PostCall)),
        Some(EventHookParameter::Single(name)) => {
            Ok(EventHookConfig::Single(GuardrailEventHook::parse(name)?))
        }
        Some(EventHookParameter::Many(names)) => names
            .iter()
            .map(|name| GuardrailEventHook::parse(name))
            .collect::<Result<Vec<GuardrailEventHook>, JudgeError>>()
            .map(EventHookConfig::Many),
        Some(EventHookParameter::Mode(mode)) => Ok(EventHookConfig::Mode(mode.clone())),
    }
}

// LiteLLM allows logging_only for any guardrail that implements apply_guardrail.
pub fn validate_event_hook(config: &EventHookConfig) -> Result<(), JudgeError> {
    for hook in config.configured_hooks()? {
        if !SUPPORTED_EVENT_HOOKS.contains(&hook) && hook != GuardrailEventHook::LoggingOnly {
            return Err(JudgeError::UnsupportedEventHook { hook });
        }
    }
    Ok(())
}

#[must_use]
pub fn event_type_for(
    config: &EventHookConfig,
    input_type: GuardrailInputType,
) -> Option<GuardrailEventHook> {
    let lifecycle: &[GuardrailEventHook] = match input_type {
        GuardrailInputType::Request => &REQUEST_LIFECYCLE,
        GuardrailInputType::Response => &RESPONSE_LIFECYCLE,
    };
    let mut matching = lifecycle
        .iter()
        .copied()
        .filter(|hook| config.is_configured(*hook));
    let first = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    Some(first)
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct GuardrailCallData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub litellm_params: Option<GuardrailValue>,
    #[serde(flatten)]
    pub rest: BTreeMap<String, GuardrailValue>,
}

#[must_use]
pub fn is_logged_judge_call(data: &GuardrailCallData, event_type: GuardrailEventHook) -> bool {
    if event_type != GuardrailEventHook::LoggingOnly {
        return false;
    }
    let Some(GuardrailValue::Object(params)) = &data.litellm_params else {
        return false;
    };
    let Some(GuardrailValue::Object(metadata)) = params.get("metadata") else {
        return false;
    };
    matches!(
        metadata.get(INTERNAL_CALL_ORIGIN),
        Some(GuardrailValue::Text(origin)) if origin == JUDGE_CALL_ORIGIN
    )
}

#[must_use]
pub fn should_run_guardrail(
    data: &GuardrailCallData,
    event_type: GuardrailEventHook,
    base: fn(&GuardrailCallData, GuardrailEventHook) -> bool,
) -> bool {
    if is_logged_judge_call(data, event_type) {
        return false;
    }
    base(data, event_type)
}
