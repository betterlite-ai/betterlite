use std::collections::BTreeMap;

use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::sso_types::exposed_secret::ExposedSecret;
use crate::user_roles::UserRole;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SsoConfig {
    pub google_client_id: Option<String>,
    #[serde(
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<ExposedSecret>>>::serialize"
    )]
    pub google_client_secret: Option<SecretString>,
    pub microsoft_client_id: Option<String>,
    #[serde(
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<ExposedSecret>>>::serialize"
    )]
    pub microsoft_client_secret: Option<SecretString>,
    pub microsoft_tenant: Option<String>,
    pub generic_client_id: Option<String>,
    #[serde(
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<ExposedSecret>>>::serialize"
    )]
    pub generic_client_secret: Option<SecretString>,
    pub generic_authorization_endpoint: Option<String>,
    pub generic_token_endpoint: Option<String>,
    pub generic_userinfo_endpoint: Option<String>,
    pub generic_scope: Option<String>,
    pub saml_idp_metadata_url: Option<String>,
    pub saml_idp_metadata_xml: Option<String>,
    pub saml_sp_entity_id: Option<String>,
    pub saml_allow_unsolicited: Option<String>,
    pub proxy_base_url: Option<String>,
    pub user_email: Option<String>,
    pub ui_access_mode: Option<UiAccessMode>,
    pub role_mappings: Option<RoleMappings>,
    pub team_mappings: Option<TeamMappings>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RoleMappings {
    pub provider: String,
    pub group_claim: String,
    pub default_role: Option<UserRole>,
    #[serde(default)]
    pub roles: RoleGroups,
}

#[derive(Debug, Error)]
pub enum RoleGroupsError {
    #[error("`{role}` is not a LiteLLM user role")]
    UnknownRole { role: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(
    try_from = "BTreeMap<String, Vec<String>>",
    into = "BTreeMap<String, Vec<String>>"
)]
pub struct RoleGroups {
    pub entries: Vec<(UserRole, Vec<String>)>,
}

impl TryFrom<BTreeMap<String, Vec<String>>> for RoleGroups {
    type Error = RoleGroupsError;

    fn try_from(groups: BTreeMap<String, Vec<String>>) -> Result<Self, Self::Error> {
        let mut entries = Vec::with_capacity(groups.len());
        for (name, members) in groups {
            let role = name
                .parse::<UserRole>()
                .map_err(|_unrecognized| RoleGroupsError::UnknownRole { role: name })?;
            entries.push((role, members));
        }
        Ok(Self { entries })
    }
}

impl From<RoleGroups> for BTreeMap<String, Vec<String>> {
    fn from(groups: RoleGroups) -> Self {
        groups
            .entries
            .into_iter()
            .map(|(role, members)| (role.to_string(), members))
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct TeamMappings {
    pub team_ids_jwt_field: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum UiAccessMode {
    Restricted(AccessControlUiAccessMode),
    Value(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct AccessControlUiAccessMode {
    #[serde(rename = "type")]
    pub mode_type: UiAccessModeType,
    pub restricted_sso_group: String,
    pub sso_group_jwt_field: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UiAccessModeType {
    RestrictedSsoGroup,
}

#[must_use]
pub fn check_is_admin_only_access(mode: &UiAccessMode) -> bool {
    match mode {
        UiAccessMode::Restricted(_restriction) => false,
        UiAccessMode::Value(text) => text == "admin_only",
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct DefaultTeamSsoParams {
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub max_budget: Option<Decimal>,
    pub budget_duration: Option<String>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
    pub team_member_permissions: Option<Vec<String>>,
    pub organization_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct UpperboundKeyGenerateParams {
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub max_budget: Option<Decimal>,
    pub budget_duration: Option<String>,
    pub duration: Option<String>,
    pub max_parallel_requests: Option<i64>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct MicrosoftGraphDirectoryObject {
    pub odata_type: Option<String>,
    pub id: Option<String>,
    #[serde(rename = "deletedDateTime")]
    pub deleted_date_time: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(rename = "roleTemplateId")]
    pub role_template_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct MicrosoftGraphGroupResponse {
    pub odata_context: Option<String>,
    #[serde(rename = "odata_nextLink")]
    pub odata_next_link: Option<String>,
    pub value: Option<Vec<MicrosoftGraphDirectoryObject>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct MicrosoftServicePrincipalTeam {
    #[serde(rename = "principalDisplayName")]
    pub principal_display_name: Option<String>,
    #[serde(rename = "principalId")]
    pub principal_id: Option<String>,
}
