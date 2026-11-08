pub mod json_value;

mod tests;

use std::error::Error;

use serde::{Deserialize, Serialize};

use self::json_value::JsonValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", try_from = "String")]
pub enum PipelineAction {
    Allow,
    Block,
    Next,
    ModifyResponse,
}

impl TryFrom<String> for PipelineAction {
    type Error = PipelineError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "allow" => Ok(Self::Allow),
            "block" => Ok(Self::Block),
            "next" => Ok(Self::Next),
            "modify_response" => Ok(Self::ModifyResponse),
            _ => Err(PipelineError::Action { value }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", try_from = "String")]
pub enum PipelineMode {
    PreCall,
    PostCall,
}

impl TryFrom<String> for PipelineMode {
    type Error = PipelineError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "pre_call" => Ok(Self::PreCall),
            "post_call" => Ok(Self::PostCall),
            _ => Err(PipelineError::Mode { value }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    Pass,
    Fail,
    Error,
}

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error(
        "Invalid action '{value}'. Must be one of: ['allow', 'block', 'modify_response', 'next']"
    )]
    Action { value: String },
    #[error("Invalid mode '{value}'. Must be one of: ['post_call', 'pre_call']")]
    Mode { value: String },
    #[error("List should have at least 1 item after validation, not 0")]
    NoSteps,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineStep {
    pub guardrail: String,
    #[serde(default = "default_on_fail")]
    pub on_fail: PipelineAction,
    #[serde(default = "default_on_pass")]
    pub on_pass: PipelineAction,
    pub on_error: Option<PipelineAction>,
    #[serde(default)]
    pub pass_data: bool,
    pub modify_response_message: Option<String>,
}

const fn default_on_fail() -> PipelineAction {
    PipelineAction::Block
}

const fn default_on_pass() -> PipelineAction {
    PipelineAction::Allow
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawGuardrailPipeline")]
pub struct GuardrailPipeline {
    pub mode: PipelineMode,
    pub steps: Vec<PipelineStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGuardrailPipeline {
    mode: PipelineMode,
    steps: Vec<PipelineStep>,
}

impl TryFrom<RawGuardrailPipeline> for GuardrailPipeline {
    type Error = PipelineError;

    fn try_from(raw: RawGuardrailPipeline) -> Result<Self, Self::Error> {
        let RawGuardrailPipeline { mode, steps } = raw;
        if steps.is_empty() {
            return Err(PipelineError::NoSteps);
        }
        Ok(Self { mode, steps })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineStepResult {
    pub guardrail_name: String,
    pub outcome: StepOutcome,
    pub action_taken: PipelineAction,
    pub modified_data: Option<JsonValue>,
    pub error_detail: Option<String>,
    pub duration_seconds: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineExecutionResult {
    pub terminal_action: PipelineAction,
    pub step_results: Vec<PipelineStepResult>,
    pub modified_data: Option<JsonValue>,
    pub error_message: Option<String>,
    pub modify_response_message: Option<String>,
    #[serde(skip)]
    pub original_exception: Option<Box<dyn Error + Send + Sync>>,
}
