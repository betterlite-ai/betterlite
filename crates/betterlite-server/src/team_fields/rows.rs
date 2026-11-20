use std::collections::BTreeMap;
use std::fmt::{Debug, Display};

use async_trait::async_trait;
use betterlite_auth::auth_invalidation::AuthInvalidation;
use betterlite_auth::auth_invalidation::error::InvalidationError;
use betterlite_config::sections::settings::JsonValue;
use betterlite_config::sections::team_admin_fields::{
    EDITABLE_FIELDS_SETTING, SUPPORTED_TEAM_FIELDS, resolve_editable_fields,
};
use betterlite_db::team_repository::decoder::{TeamRecord, TeamRecordError, decode_team_record};
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
pub struct GeneralSettingsMap {
    pub entries: BTreeMap<String, JsonValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("general_settings must be a mapping of setting names to values")]
pub struct SettingsMapError;

impl GeneralSettingsMap {
    pub fn from_value(value: &JsonValue) -> Result<Self, SettingsMapError> {
        let JsonValue::Object(entries) = value else {
            return Err(SettingsMapError);
        };
        let entries = entries
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        Ok(Self { entries })
    }
}

#[derive(Clone, Copy)]
pub enum LogValue<'value> {
    Display(&'value dyn Display),
    Debug(&'value dyn Debug),
}

// LiteLLM strips every CR and LF so a user-controlled value cannot forge a log line.
#[must_use]
pub fn sanitize_for_log(value: LogValue<'_>) -> String {
    let text = match value {
        LogValue::Display(value) => value.to_string(),
        LogValue::Debug(value) => format!("{value:?}"),
    };
    text.replace('\r', "").replace('\n', "")
}

#[async_trait]
pub trait TeamObjectCache: Send + Sync {
    async fn store_team(
        &self,
        team_id: &str,
        team: &mut LiteLLMTeamTableCachedObj,
    ) -> Result<(), InvalidationError>;
}

#[async_trait]
impl TeamObjectCache for AuthInvalidation {
    async fn store_team(
        &self,
        team_id: &str,
        team: &mut LiteLLMTeamTableCachedObj,
    ) -> Result<(), InvalidationError> {
        self.cache_team_object(team_id, team).await
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    #[error("the returned team row cannot be decoded")]
    Row(#[from] TeamRecordError),
    #[error("the refreshed team object cannot be cached")]
    Cache(#[from] InvalidationError),
}

pub async fn refresh_cached_team(
    team_row: &TeamRecord,
    cache: &dyn TeamObjectCache,
) -> Result<(), RefreshError> {
    let team = decode_team_record(Some(team_row))?.ok_or(TeamRecordError::NotAnObject)?;
    let mut cached = LiteLLMTeamTableCachedObj {
        team,
        last_refreshed_at: None,
    };
    let team_id = cached.team.fields.base.team_id.clone();
    cache.store_team(&team_id, &mut cached).await?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallerRole {
    ProxyAdmin,
    OrgAdmin,
    TeamAdmin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TeamEditAccess {
    Unrestricted,
    TeamAdmin { editable_fields: Vec<String> },
    TeamAdminDisabled,
    None,
}

#[must_use]
pub fn caller_edit_access(
    role: Option<CallerRole>,
    general_settings: &GeneralSettingsMap,
) -> TeamEditAccess {
    match role {
        Some(CallerRole::ProxyAdmin | CallerRole::OrgAdmin) => TeamEditAccess::Unrestricted,
        Some(CallerRole::TeamAdmin) => team_admin_access(general_settings),
        None => TeamEditAccess::None,
    }
}

fn team_admin_access(general_settings: &GeneralSettingsMap) -> TeamEditAccess {
    let permitted = resolve_editable_fields(
        general_settings.entries.get(EDITABLE_FIELDS_SETTING),
        &SUPPORTED_TEAM_FIELDS,
    );
    if permitted.fields.is_empty() {
        return TeamEditAccess::TeamAdminDisabled;
    }
    TeamEditAccess::TeamAdmin {
        editable_fields: permitted.fields.into_iter().collect(),
    }
}
