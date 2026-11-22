use betterlite_auth::identity::{EntraIdentityConfig, ExecutionMode};
use betterlite_core::agents::AgentResponse;
use betterlite_core::agents::json::AgentJson;

use super::error::{Detail, IdentityAdminError};
use super::issuers::trusted_agent_issuers;

pub(crate) use super::issuers::IdentitySettings;

mod tests;

// the request key can be missing, null or set; the three differ, and B4785 builds this from the body
#[derive(Clone, Debug)]
pub(crate) enum IdentityField {
    Absent,
    Null,
    Value(AgentJson),
}

#[derive(Clone, Debug)]
pub(crate) struct IdentityRequest {
    pub(crate) identity: IdentityField,
    pub(crate) execution_mode: Option<ExecutionMode>,
}

// ast-grep-ignore: no-long-names
pub(crate) fn validate_managed_identity_request(
    request: &IdentityRequest,
    existing: Option<&AgentResponse>,
    settings: &IdentitySettings,
) -> Result<(), IdentityAdminError> {
    let (issuer, tenant_id) = match &request.identity {
        IdentityField::Null => return Ok(()),
        IdentityField::Value(value) => {
            let identity = entra_identity(value)?;
            (identity.issuer(), identity.tenant_id.to_string())
        }
        IdentityField::Absent => match existing.and_then(|agent| agent.identity.as_ref()) {
            Some(binding) => (binding.issuer.clone(), binding.tenant_id.to_string()),
            None => return Ok(()),
        },
    };
    if !trusted_agent_issuers(settings).contains(&issuer) {
        return Err(IdentityAdminError {
            status: 400,
            detail: Detail::Text(String::from(
                "Configure trusted JWT issuer and audience validation for this Entra tenant first",
            )),
        });
    }
    let mode = request
        .execution_mode
        .clone()
        .or_else(|| existing.map(|agent| agent.execution_mode.clone()))
        .unwrap_or_default();
    if mode != ExecutionMode::Autonomous && !microsoft_sso_matches(settings, &tenant_id) {
        return Err(IdentityAdminError {
            status: 400,
            detail: Detail::Text(String::from(
                "Delegated agents require Microsoft SSO for the same trusted tenant",
            )),
        });
    }
    Ok(())
}

fn entra_identity(value: &AgentJson) -> Result<EntraIdentityConfig, IdentityAdminError> {
    serde_json::from_value(value.clone()).map_err(|_error| IdentityAdminError {
        status: 400,
        detail: Detail::Text(String::from("Invalid Entra identity configuration")),
    })
}

fn microsoft_sso_matches(settings: &IdentitySettings, tenant_id: &str) -> bool {
    settings.microsoft_tenant.as_deref() == Some(tenant_id)
        && settings
            .microsoft_client_id
            .as_deref()
            .is_some_and(|client_id| !client_id.is_empty())
}
