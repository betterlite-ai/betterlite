use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use crate::request_body_safety::SafetyError;
use crate::request_body_safety::metadata;

pub const GUARDRAIL_DENIAL: &str = "Your team does not have permission to modify guardrails.";

const GUARDRAIL_MODIFICATION_KEYS: [&str; 4] = [
    "guardrails",
    "disable_global_guardrails",
    "disable_global_guardrail",
    "opted_out_global_guardrails",
];

#[derive(Debug, thiserror::Error)]
pub enum GuardrailCheckError {
    #[error("{GUARDRAIL_DENIAL}")]
    Denied,
    #[error(transparent)]
    Metadata(#[from] SafetyError),
}

// _guardrail_modification_check: any guardrail key in the two metadata containers or the root is a modification
pub fn guardrail_modification_check(
    request_body: &JsonObject,
    team: Option<&LiteLLMTeamTable>,
) -> Result<(), GuardrailCheckError> {
    let modifies = metadata_requested_modification(request_body.get("metadata"))?
        || metadata_requested_modification(request_body.get("litellm_metadata"))?
        || GUARDRAIL_MODIFICATION_KEYS
            .iter()
            .any(|key| request_body.contains_key(*key));
    if !modifies {
        return Ok(());
    }
    if can_modify_guardrails(team) {
        return Ok(());
    }
    Err(GuardrailCheckError::Denied)
}

// a dict counts, a JSON-string object counts, and malformed or non-object containers are ignored
fn metadata_requested_modification(container: Option<&Json>) -> Result<bool, SafetyError> {
    let Some(fields) = metadata::coerce(container)? else {
        return Ok(false);
    };
    Ok(GUARDRAIL_MODIFICATION_KEYS
        .iter()
        .any(|key| fields.contains_key(*key)))
}

// can_modify_guardrails: only a dict team.metadata.guardrails with modify_guardrails literally False denies
#[must_use]
pub fn can_modify_guardrails(team: Option<&LiteLLMTeamTable>) -> bool {
    let Some(metadata) = team.and_then(|row| row.fields.base.metadata.as_ref()) else {
        return true;
    };
    let Some(JsonData::Object(guardrails)) = metadata.get("guardrails") else {
        return true;
    };
    !matches!(
        guardrails.get("modify_guardrails"),
        Some(JsonData::Bool(false))
    )
}

impl GuardrailCheckError {
    #[must_use]
    pub fn into_auth_failure(self) -> AuthFailure {
        match self {
            Self::Denied => AuthFailure::Http {
                detail: GUARDRAIL_DENIAL.to_owned(),
                param: None,
                status_code: 403,
                invalid_virtual_key: false,
            },
            Self::Metadata(error) => AuthFailure::Other {
                message: error.to_string(),
                param: None,
            },
        }
    }

    #[must_use]
    pub fn into_proxy_exception(self) -> ProxyException {
        as_proxy_exception(&self.into_auth_failure())
            .exception()
            .clone()
    }
}
