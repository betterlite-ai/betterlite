use std::collections::BTreeMap;

use secrecy::SecretString;

use crate::context::SessionContext;
use crate::errors::HarnessError;
use crate::types::Harness;

use super::require_model;

#[derive(Clone, Debug, Default)]
pub struct ChatModelKwargs {
    pub model: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub extra_headers: Option<BTreeMap<String, String>>,
}

const fn harness_value(harness: Harness) -> &'static str {
    match harness {
        Harness::ClaudeCode => "claude_code",
        Harness::Codex => "codex",
        Harness::OpenCode => "opencode",
        Harness::DeepAgents => "deepagents",
    }
}

pub fn gateway_headers<S>(
    context: &SessionContext<S>,
) -> Result<BTreeMap<String, String>, HarnessError> {
    let mut headers = BTreeMap::from([(
        "x-litellm-tags".to_owned(),
        format!("harness,{}", harness_value(context.harness)),
    )]);
    if !context.settings.metadata.is_empty() {
        let metadata = serde_json::to_string(&context.settings.metadata)
            .map_err(|source| HarnessError::StateEncode { source })?;
        headers.insert("x-litellm-spend-logs-metadata".to_owned(), metadata);
    }
    Ok(headers)
}

pub fn chat_model_kwargs<S>(context: &SessionContext<S>) -> Result<ChatModelKwargs, HarnessError> {
    let model = require_model(context)?;
    match context.settings.gateway.as_ref() {
        Some(gateway) => Ok(ChatModelKwargs {
            model: format!("litellm_proxy/{model}"),
            api_base: Some(gateway.api_base.clone()),
            api_key: Some(gateway.api_key.clone()),
            extra_headers: Some(gateway_headers(context)?),
        }),
        None => Ok(ChatModelKwargs {
            model: model.to_owned(),
            api_base: context.settings.api_base.clone(),
            api_key: context.settings.api_key.clone(),
            extra_headers: None,
        }),
    }
}
