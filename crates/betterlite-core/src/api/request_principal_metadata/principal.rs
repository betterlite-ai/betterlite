use betterlite_auth::agent_policy::ManagedAgentMode;
use betterlite_models::decisions::Json;
use chrono::SecondsFormat;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::api::request_destination::RequestData;
use crate::api::request_metadata_safety::metadata::MetadataField;
use crate::api::request_principal_metadata::error::RequestPrincipalMetadataError;
use crate::api::request_principal_metadata::{Metadata, PrincipalFacts, take_bucket};

// the metadata slots that carry callback credentials, which no logged or forwarded copy keeps
pub const CALLBACK_CONFIG_SLOTS: [&str; 3] =
    ["logging", "callback_settings", "secret_manager_settings"];
const AGENT_ID_KEY: &str = "agent_id";

// StandardLoggingUserAPIKeyMetadata: the key, team, project and user facts every logged request carries
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserKeyLogMetadata {
    pub user_api_key_hash: Option<String>,
    pub user_api_key_alias: Option<String>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub user_api_key_spend: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub user_api_key_max_budget: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub user_api_key_user_spend: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub user_api_key_user_max_budget: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub user_api_key_team_spend: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub user_api_key_team_max_budget: Option<Decimal>,
    pub user_api_key_team_id: Option<String>,
    pub user_api_key_project_id: Option<String>,
    pub user_api_key_project_alias: Option<String>,
    pub user_api_key_user_id: Option<String>,
    pub user_api_key_org_id: Option<String>,
    pub user_api_key_org_alias: Option<String>,
    pub user_api_key_team_alias: Option<String>,
    pub user_api_key_end_user_id: Option<String>,
    pub user_api_key_user_email: Option<String>,
    pub user_api_key_request_route: Option<String>,
    pub user_api_key_budget_reset_at: Option<String>,
    pub user_api_key_auth_metadata: Metadata,
}

impl From<&PrincipalFacts> for UserKeyLogMetadata {
    fn from(principal: &PrincipalFacts) -> Self {
        Self {
            user_api_key_hash: logged_api_key(principal).map(str::to_owned),
            user_api_key_alias: principal.key_alias.clone(),
            user_api_key_spend: principal.spend,
            user_api_key_max_budget: principal.max_budget,
            user_api_key_user_spend: principal.user_spend,
            user_api_key_user_max_budget: principal.user_max_budget,
            user_api_key_team_spend: principal.team_spend,
            user_api_key_team_max_budget: principal.team_max_budget,
            user_api_key_team_id: principal.team_id.clone(),
            user_api_key_project_id: principal.project_id.clone(),
            user_api_key_project_alias: principal.project_alias.clone(),
            user_api_key_user_id: principal.user_id.clone(),
            user_api_key_org_id: principal.org_id.clone(),
            user_api_key_org_alias: principal.organization_alias.clone(),
            user_api_key_team_alias: principal.team_alias.clone(),
            user_api_key_end_user_id: principal.end_user_id.clone(),
            user_api_key_user_email: principal.user_email.clone(),
            user_api_key_request_route: principal.request_route.clone(),
            user_api_key_budget_reset_at: principal
                .budget_reset_at
                .map(|reset_at| reset_at.to_rfc3339_opts(SecondsFormat::AutoSi, false)),
            user_api_key_auth_metadata: strip_callback_config(&principal.metadata),
        }
    }
}

// everything add_user_api_key_auth_to_request_metadata writes except agent_id, which reads the slot first
#[derive(Serialize)]
struct PrincipalEntries<'principal> {
    #[serde(flatten)]
    logged: UserKeyLogMetadata,
    user_api_key: Option<&'principal str>,
    actor_agent_id: Option<&'principal str>,
    target_agent_id: Option<&'principal str>,
    billing_agent_id: Option<&'principal str>,
    agent_execution_mode: Option<&'principal ManagedAgentMode>,
    verified_human_user_id: Option<&'principal str>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    user_api_end_user_max_budget: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_api_key_budget_reservation: Option<&'principal Metadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_api_key_matched_model_access_groups: Option<&'principal [String]>,
}

// a session token logs under its key alias so the dashboard session never shows up as a key hash
#[must_use]
pub fn logged_api_key(principal: &PrincipalFacts) -> Option<&str> {
    if principal.is_session_token
        && let Some(alias) = &principal.key_alias
    {
        return Some(alias);
    }
    principal.api_key_hash.as_deref()
}

#[must_use]
pub fn strip_callback_config(metadata: &Metadata) -> Metadata {
    let mut stripped = metadata.clone();
    stripped.retain(|key, _value| !CALLBACK_CONFIG_SLOTS.contains(&key.as_str()));
    stripped
}

pub fn add_principal_metadata(
    data: &mut RequestData,
    slot: MetadataField,
    principal: &PrincipalFacts,
) -> Result<PrincipalFacts, RequestPrincipalMetadataError> {
    let managed = principal.managed_agent_context.as_ref();
    let entries = PrincipalEntries {
        logged: UserKeyLogMetadata::from(principal),
        user_api_key: logged_api_key(principal),
        actor_agent_id: principal.agent_id.as_deref(),
        target_agent_id: principal.invoked_agent_id.as_deref(),
        billing_agent_id: principal
            .agent_id
            .as_deref()
            .or(principal.invoked_agent_id.as_deref()),
        agent_execution_mode: managed.map(|context| &context.mode),
        verified_human_user_id: managed.and_then(|context| context.user_id.as_deref()),
        user_api_end_user_max_budget: principal.end_user_max_budget,
        user_api_key_budget_reservation: principal.budget_reservation.as_ref(),
        user_api_key_matched_model_access_groups: (!principal
            .matched_model_access_groups
            .is_empty())
        .then_some(principal.matched_model_access_groups.as_slice()),
    };
    let encoded = serde_json::to_string(&entries)
        .map_err(|source| RequestPrincipalMetadataError::Encode { source })?;
    let written: Metadata = serde_json::from_str(&encoded)
        .map_err(|source| RequestPrincipalMetadataError::Encode { source })?;
    let mut bucket = take_bucket(data, slot);
    // a header may have named the agent already, and the key's own or invoked agent outranks it
    let agent_id = principal
        .invoked_agent_id
        .as_ref()
        .or(principal.agent_id.as_ref())
        .map_or_else(
            || bucket.get(AGENT_ID_KEY).cloned().unwrap_or(Json::Null),
            |agent| Json::Text(agent.clone()),
        );
    bucket.extend(written);
    bucket.insert(AGENT_ID_KEY.to_owned(), agent_id);
    data.insert(slot.as_str().to_owned(), Json::Object(bucket));
    Ok(stripped_principal(principal))
}

// the principal as later MCP access checks read it, with every metadata map stripped
#[must_use]
pub fn stripped_principal(principal: &PrincipalFacts) -> PrincipalFacts {
    let mut auth_view = principal.clone();
    auth_view.metadata = strip_callback_config(&principal.metadata);
    auth_view.team_metadata = principal.team_metadata.as_ref().map(strip_callback_config);
    auth_view.project_metadata = principal
        .project_metadata
        .as_ref()
        .map(strip_callback_config);
    auth_view.organization_metadata = principal
        .organization_metadata
        .as_ref()
        .map(strip_callback_config);
    auth_view
}
