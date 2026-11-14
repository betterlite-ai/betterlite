use std::time::SystemTime;

use betterlite_config::sections::content_filter_records::BlockCodeAction;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::block_code_execution::block_detection::{BlockDetector, FENCED_BLOCK_PATTERN};
use crate::block_code_execution::text_scanning::CodeBlockScanner;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::error::GuardrailBaseError;
use crate::guardrail_base::hooks::validate_event_hook;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};

pub const DEFAULT_GUARDRAIL_NAME: &str = "block_code_execution";

pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 3] = [
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::DuringCall,
];

pub const DEFAULT_EVENT_HOOKS: [GuardrailEventHooks; 2] =
    [GuardrailEventHooks::PreCall, GuardrailEventHooks::PostCall];

// every valid wire name, so a typo fails here whatever LITELLM_STRICT_GUARDRAIL_MODES says
const ALL_EVENT_HOOKS: [GuardrailEventHooks; 8] = [
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::LoggingOnly,
    GuardrailEventHooks::PreMcpCall,
    GuardrailEventHooks::DuringMcpCall,
    GuardrailEventHooks::PostMcpCall,
    GuardrailEventHooks::RealtimeInputTranscription,
];

const BLOCK_CODE_EXECUTION_SWITCHES: GuardrailSwitches = GuardrailSwitches {
    use_native_during_call_hook: false,
    use_native_lifecycle_hooks: false,
    records_own_information: false,
    uses_apply_guardrail_interface: true,
    deny_text_on_error: false,
};

// LiteLLM's Literal["pre_call", "post_call", "during_call"] | list[str]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EventHookSelection {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct BlockCodeExecutionParams {
    pub guardrail_name: Option<String>,
    pub blocked_languages: Option<Vec<String>>,
    pub action: BlockCodeAction,
    pub confidence_threshold: f64,
    pub detect_execution_intent: bool,
    pub event_hook: Option<EventHookSelection>,
    pub default_on: bool,
    pub clock: fn() -> SystemTime,
}

impl Default for BlockCodeExecutionParams {
    fn default() -> Self {
        Self {
            guardrail_name: None,
            blocked_languages: None,
            action: BlockCodeAction::Block,
            confidence_threshold: 0.5,
            detect_execution_intent: true,
            event_hook: None,
            default_on: false,
            clock: SystemTime::now,
        }
    }
}

#[derive(Debug, Error)]
pub enum BlockCodeExecutionError {
    #[error(transparent)]
    Base(#[from] GuardrailBaseError),
    #[error("the fenced code block pattern is invalid")]
    Pattern {
        #[source]
        source: regex::Error,
    },
}

pub struct BlockCodeExecutionGuardrail {
    pub base: GuardrailBase,
    pub blocked_languages: Option<Vec<String>>,
    pub block_all: bool,
    pub scanner: CodeBlockScanner,
    pub clock: fn() -> SystemTime,
}

impl BlockCodeExecutionGuardrail {
    pub fn new(
        params: BlockCodeExecutionParams,
        strict_modes: bool,
    ) -> Result<Self, BlockCodeExecutionError> {
        let scanner = code_block_scanner(&params)?;
        let name = match params.guardrail_name {
            Some(name) if !name.is_empty() => name,
            Some(_) | None => DEFAULT_GUARDRAIL_NAME.to_owned(),
        };
        let mode = event_hook_mode(params.event_hook.as_ref());
        validate_event_hook(
            Some(&mode),
            &ALL_EVENT_HOOKS,
            &BLOCK_CODE_EXECUTION_SWITCHES,
        )?;
        let base = GuardrailBase::new(
            guardrail_settings(name, mode, params.default_on),
            strict_modes,
        )?;
        let block_all = params.blocked_languages.as_ref().is_none_or(Vec::is_empty);
        Ok(Self {
            base,
            blocked_languages: params.blocked_languages,
            block_all,
            scanner,
            clock: params.clock,
        })
    }
}

fn guardrail_settings(
    name: String,
    mode: LitellmParamsMode,
    default_on: bool,
) -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: Some(name),
        supported_event_hooks: Some(SUPPORTED_EVENT_HOOKS.to_vec()),
        event_hook: Some(mode),
        default_on,
        mask_request_content: false,
        mask_response_content: false,
        violation_message_template: None,
        end_session_after_n_fails: None,
        on_violation: None,
        realtime_violation_message: None,
        on_sensitive_data: None,
        sensitive_data_route_to_model: None,
        sticky_session_routing: true,
        run_in_parallel: false,
        scan_raw_request: false,
        only_scan_new_messages: false,
        timeout: None,
        switches: BLOCK_CODE_EXECUTION_SWITCHES,
    }
}

fn code_block_scanner(
    params: &BlockCodeExecutionParams,
) -> Result<CodeBlockScanner, BlockCodeExecutionError> {
    let fenced_block = Regex::new(FENCED_BLOCK_PATTERN)
        .map_err(|source| BlockCodeExecutionError::Pattern { source })?;
    Ok(CodeBlockScanner {
        detector: BlockDetector {
            fenced_block,
            blocked_languages: params.blocked_languages.clone().unwrap_or_default(),
            confidence_threshold: params.confidence_threshold.clamp(0.0, 1.0),
        },
        action: params.action,
        detect_execution_intent: params.detect_execution_intent,
    })
}

fn event_hook_mode(selection: Option<&EventHookSelection>) -> LitellmParamsMode {
    match selection {
        None => LitellmParamsMode::Many(vec!["pre_call".to_owned(), "post_call".to_owned()]),
        Some(EventHookSelection::One(name)) => LitellmParamsMode::One(name.clone()),
        Some(EventHookSelection::Many(names)) => LitellmParamsMode::Many(names.clone()),
    }
}
