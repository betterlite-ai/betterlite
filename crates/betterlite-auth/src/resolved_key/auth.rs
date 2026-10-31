use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::identity_rows::Payload;
use betterlite_db::identity_rows::end_user::Region;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use crate::agent_policy::caller::AgentCaller;
use crate::agent_policy::{AgentResponse, ManagedAgentContext};
use crate::budget_snapshots::{OrgBudgetSnapshot, TeamBudgetSnapshot, UserBudgetSnapshot};
use crate::resolved_key::materialize;
use crate::resolved_key::view::{VerificationTokenView, ViewError};
use crate::user_roles::UserRole;

mod tests;

const STRIPPED_FIELDS: [&str; 15] = [
    "mcp_admitted_user_subject",
    "requires_fresh_policy",
    "mcp_explicit_grants_only",
    "mcp_source_team_rpm_limits",
    "mcp_session_resource_server_id",
    "mcp_toolset_id",
    "via_virtual_key",
    "authenticated_by_custom_auth",
    "agent_caller",
    "managed_agent_context",
    "managed_agent_policy",
    "invoked_agent_id",
    "invoked_agent_policy",
    "agent_invocation_cost",
    "billing_agent_policy",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ParentOtelSpan {
    Json(JsonData),
    #[serde(skip)]
    Opaque,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct McpAccessMarkers {
    #[serde(skip_serializing)]
    pub(crate) mcp_admitted_user_subject: bool,
    #[serde(skip_serializing)]
    pub(crate) mcp_explicit_grants_only: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct CredentialProvenanceMarkers {
    #[serde(skip_serializing)]
    pub(crate) via_virtual_key: bool,
    #[serde(skip_serializing)]
    pub(crate) authenticated_by_custom_auth: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(transparent)]
pub(crate) struct ApiKeySecret(
    #[serde(
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<ExposedApiKey>>>::serialize"
    )]
    pub(crate) Option<SecretString>,
);

// Serde's library adapter creates this plaintext value only during explicit serialization.
#[derive(Serialize)]
#[serde(transparent)]
struct ExposedApiKey(String);

impl From<&SecretString> for ExposedApiKey {
    fn from(secret: &SecretString) -> Self {
        Self(secret.expose_secret().to_owned())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct AuthExtra {
    pub(crate) api_key: ApiKeySecret,
    pub(crate) user_role: Option<UserRole>,
    pub(crate) allowed_model_region: Option<Region>,
    pub(crate) parent_otel_span: Option<ParentOtelSpan>,
    pub(crate) rpm_limit_per_model: Option<BTreeMap<String, i64>>,
    pub(crate) tpm_limit_per_model: Option<BTreeMap<String, i64>>,
    pub(crate) user_tpm_limit: Option<i64>,
    pub(crate) user_rpm_limit: Option<i64>,
    pub(crate) user_email: Option<String>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub(crate) user_spend: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub(crate) user_max_budget: Option<Decimal>,
    pub(crate) user_model_max_budget: Option<Payload>,
    pub(crate) request_route: Option<String>,
    pub(crate) is_session_token: bool,
    pub(crate) user: Option<JsonData>,
    pub(crate) created_by_user: Option<JsonData>,
    pub(crate) end_user_object_permission: Option<LiteLLMObjectPermissionTable>,
    pub(crate) team_object_permission: Option<LiteLLMObjectPermissionTable>,
    pub(crate) jwt_claims: Option<Payload>,

    #[serde(flatten)]
    pub(crate) mcp_access: McpAccessMarkers,
    #[serde(skip_serializing)]
    pub(crate) requires_fresh_policy: bool,
    #[serde(flatten)]
    pub(crate) provenance: CredentialProvenanceMarkers,
    #[serde(skip_serializing)]
    pub(crate) mcp_source_team_rpm_limits: Option<BTreeMap<String, BTreeMap<String, i64>>>,
    #[serde(skip_serializing)]
    pub(crate) mcp_session_resource_server_id: Option<String>,
    #[serde(skip_serializing)]
    pub(crate) mcp_toolset_id: Option<String>,
    #[serde(skip_serializing)]
    pub(crate) invoked_agent_id: Option<String>,
    #[serde(skip_serializing)]
    pub(crate) invoked_agent_policy: Option<AgentResponse>,
    #[serde(
        skip_serializing,
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub(crate) agent_invocation_cost: Option<Decimal>,
    #[serde(skip_serializing)]
    pub(crate) billing_agent_policy: Option<AgentResponse>,
    #[serde(skip_serializing)]
    pub(crate) managed_agent_policy: Option<AgentResponse>,
    #[serde(skip_serializing)]
    pub(crate) managed_agent_context: Option<ManagedAgentContext>,
    #[serde(skip_serializing)]
    pub(crate) agent_caller: Option<AgentCaller>,
    #[serde(skip_serializing)]
    pub(crate) budget_reservation: Option<Payload>,
    #[serde(skip_serializing)]
    pub(crate) team_budget_snapshot: Option<TeamBudgetSnapshot>,
    #[serde(skip_serializing)]
    pub(crate) user_budget_snapshot: Option<UserBudgetSnapshot>,
    #[serde(skip_serializing)]
    pub(crate) org_budget_snapshot: Option<OrgBudgetSnapshot>,
    #[serde(skip_serializing)]
    pub(crate) matched_model_access_groups: Option<Vec<String>>,
    #[serde(
        skip_serializing,
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub(crate) budget_throttle_pct: Option<Decimal>,
}

#[derive(Clone, Debug, Serialize)]
pub struct UserAPIKeyAuth {
    #[serde(flatten)]
    pub(crate) view: VerificationTokenView,
    #[serde(flatten)]
    pub(crate) extra: AuthExtra,
    #[serde(skip)]
    pub(crate) managed_delegation_verified: bool,
}

pub enum AuthInput {
    Instance(Box<UserAPIKeyAuth>),
    Dict(BTreeMap<String, JsonData>),
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("api_key must be a string to compute its safe provenance hash")]
    NonStringApiKey,
    #[error("failed to decode a verification token view")]
    View(#[from] ViewError),
    #[error("failed to decode user api key auth extras")]
    Json(#[source] serde_json::Error),
}

impl UserAPIKeyAuth {
    pub fn construct(input: AuthInput) -> Result<Self, AuthError> {
        let mut fields = match input {
            AuthInput::Instance(existing) => return Ok(*existing),
            AuthInput::Dict(fields) => fields,
        };
        strip_server_only_fields(&mut fields);
        hash_api_key_and_token(&mut fields)?;
        let view = VerificationTokenView::construct(fields.clone())?;
        let extra = materialize(fields).map_err(AuthError::Json)?;
        Ok(Self {
            view,
            extra,
            managed_delegation_verified: false,
        })
    }

    #[must_use]
    pub const fn take_delegation_verified(&mut self) -> bool {
        let verified = self.managed_delegation_verified;
        self.managed_delegation_verified = false;
        verified
    }
}

fn strip_server_only_fields(fields: &mut BTreeMap<String, JsonData>) {
    for name in STRIPPED_FIELDS {
        fields.remove(name);
    }
}

fn hash_api_key_and_token(fields: &mut BTreeMap<String, JsonData>) -> Result<(), AuthError> {
    let Some(value) = fields.get("api_key") else {
        return Ok(());
    };
    if matches!(value, JsonData::Null) {
        return Ok(());
    }
    let JsonData::String(api_key) = value else {
        return Err(AuthError::NonStringApiKey);
    };
    let hashed = super::credential::safe_hash_api_key(api_key);
    fields.insert("token".to_owned(), JsonData::String(hashed.clone()));
    fields.insert("api_key".to_owned(), JsonData::String(hashed));
    Ok(())
}
