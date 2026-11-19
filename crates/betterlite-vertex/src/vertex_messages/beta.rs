use std::collections::BTreeSet;

use betterlite_anthropic::feature_flags::beta::tool_search_beta_header;
use betterlite_anthropic::feature_flags::messages::has_output_config;
use betterlite_anthropic::feature_flags::tools::{is_tool_search_used, is_web_search_tool_used};
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::headers::request::WEB_SEARCH_BETA;
use betterlite_anthropic::request::beta_headers::{ANTHROPIC_BETA_HEADER, BetaHeaderError};
use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

use super::PROVIDER;
use crate::vertex_claude_chat::beta::{ContextManagementError, add_context_management_betas};

// The ANTHROPIC_BETA_HEADER_VALUES entries LiteLLM's Vertex override adds itself.
pub const DANGEROUS_TOOL_USE_BETA: &str = "dangerous-tool-use-2026-09-03";
pub const PER_TURN_CONTROL_BETA: &str = "per-turn-control-2026-07-01";

const CONTEXT_MANAGEMENT: &str = "context_management";
const SAFEGUARDS: &str = "safeguards";
const TOOLS: &str = "tools";

#[derive(Debug, Error)]
pub enum BetaError {
    #[error(transparent)]
    Context(#[from] BetaHeaderError),
    #[error("context_management must be an object")]
    ContextManagementNotObject,
}

impl BetaError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Context(error) => error.status_code(),
            Self::ContextManagementNotObject => Some(400),
        }
    }
}

impl From<ContextManagementError> for BetaError {
    fn from(error: ContextManagementError) -> Self {
        match error {
            ContextManagementError::NotObject => Self::ContextManagementNotObject,
            ContextManagementError::Context(source) => Self::Context(source),
        }
    }
}

// LiteLLM's beta set: the caller's header, context edits, hosted web search, tool search, safeguards and per-turn control.
pub fn beta_tokens(
    headers: &AnthropicHeaders,
    messages: &[Json],
    optional_params: &Object,
) -> Result<BTreeSet<String>, BetaError> {
    let mut betas = existing_tokens(headers);
    if let Some(context_management) = optional_params.get(CONTEXT_MANAGEMENT) {
        add_context_management_betas(&mut betas, context_management)?;
    }
    let tools = match optional_params.get(TOOLS) {
        Some(Json::List(tools)) => tools.as_slice(),
        _ => &[],
    };
    if is_web_search_tool_used(tools) {
        betas.insert(WEB_SEARCH_BETA.to_owned());
    }
    if is_tool_search_used(tools) {
        betas.insert(tool_search_beta_header(PROVIDER).to_owned());
    }
    if matches!(optional_params.get(SAFEGUARDS), Some(value) if !matches!(value, Json::Null)) {
        betas.insert(DANGEROUS_TOOL_USE_BETA.to_owned());
    }
    if has_output_config(messages) {
        betas.insert(PER_TURN_CONTROL_BETA.to_owned());
    }
    Ok(betas)
}

// LiteLLM strips every comma-separated value and keeps the empty ones the strip leaves behind.
fn existing_tokens(headers: &AnthropicHeaders) -> BTreeSet<String> {
    let Some(existing) = headers.values.get(ANTHROPIC_BETA_HEADER) else {
        return BTreeSet::new();
    };
    if existing.is_empty() {
        return BTreeSet::new();
    }
    existing
        .split(',')
        .map(|beta| beta.trim().to_owned())
        .collect()
}
