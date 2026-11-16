use std::error::Error as StdError;

use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_auth::resolved_key::auth::user_api_key_has_admin_view;
use betterlite_prompts::registry::prompt_registry_helpers::get_base_prompt_id;
use serde::Deserialize;

use super::MASTER_KEY_SUBJECT;
use super::error::PromptReadError;

// LiteLLM's user_api_key_has_admin_view, and the master key, which carries no key record to read a role from
pub(super) fn has_admin_view(principal: &Principal) -> bool {
    principal.subject == MASTER_KEY_SUBJECT
        || principal
            .source_key
            .0
            .as_ref()
            .is_some_and(user_api_key_has_admin_view)
        || principal.roles.contains(&Role::PlatformAdmin)
        || principal.roles.contains(&Role::PlatformViewer)
}

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
    let allowed = view
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.prompts.as_deref());
    if let Some(allowed) = allowed
        && !allowed.iter().any(|entry| entry.as_str() == prompt_id)
    {
        return Err(PromptReadError::Denied {
            status: StatusCode::BAD_REQUEST,
            detail: format!("Prompt {prompt_id} not found"),
        });
    }
    if !has_admin_view(principal) {
        return Err(PromptReadError::Denied {
            status: StatusCode::FORBIDDEN,
            detail: "You are not authorized to access this prompt".to_owned(),
        });
    }
    Ok(())
}
