use std::error::Error as StdError;

use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_prompts::registry::prompt_registry_helpers::get_base_prompt_id;
use serde::Deserialize;

use super::error::PromptReadError;
use crate::management_guards::caller::caller_has_admin_view;

// The key metadata the prompt routes read, taken from the key's serde form
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct PromptKeyView {
    pub(super) metadata: Option<PromptKeyMetadata>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct PromptKeyMetadata {
    pub(super) prompts: Option<Vec<String>>,
}

impl PromptKeyView {
    pub(super) fn from_principal(principal: &Principal) -> Result<Self, serde_json::Error> {
        let Some(key) = principal.source_key.0.as_ref() else {
            return Ok(Self::default());
        };
        serde_json::from_value(serde_json::to_value(key)?)
    }
}

// LiteLLM lists a prompt when the key's allowlist names it by exact id or by the base id without its version
pub(super) fn names_prompt(allowlist: &[String], prompt_id: &str) -> bool {
    let base = get_base_prompt_id(prompt_id);
    allowlist
        .iter()
        .any(|entry| entry == prompt_id || *entry == base)
}

// LiteLLM answers 400 for a prompt the key's allowlist omits before it checks admin-view
pub(super) fn info_access<TableError>(
    principal: &Principal,
    prompt_id: &str,
) -> Result<(), PromptReadError<TableError>>
where
    TableError: StdError + 'static,
{
    let view = PromptKeyView::from_principal(principal).map_err(PromptReadError::KeyMetadata)?;
    let allowlist = view
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.prompts.as_deref());
    if let Some(allowed) = allowlist
        && !allowed.iter().any(|entry| entry.as_str() == prompt_id)
    {
        return Err(PromptReadError::Denied {
            status: StatusCode::BAD_REQUEST,
            detail: format!("Prompt {prompt_id} not found"),
        });
    }
    if !caller_has_admin_view(principal) {
        return Err(PromptReadError::Denied {
            status: StatusCode::FORBIDDEN,
            detail: "You are not authorized to access this prompt".to_owned(),
        });
    }
    Ok(())
}
