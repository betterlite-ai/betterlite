use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_repository::repository::TeamQueryError;
use thiserror::Error;

use super::caller::CallerAccess;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SettingsReadError {
    #[error("the caller is not allowed to read general settings")]
    NotAllowed,
    #[error("the general settings store is unavailable")]
    Unavailable,
}

// the caller's role picks the read and a view-only admin gets secret values redacted; Ok(None) is an absent field
#[async_trait]
pub trait GeneralSettingsReader: Send + Sync {
    async fn general_setting(
        &self,
        name: &str,
        access: CallerAccess,
    ) -> Result<Option<JsonData>, SettingsReadError>;
}

// the pass-through entries the loaded config file holds, before any database row is merged in
pub trait ConfigEndpointsSource: Send + Sync {
    fn configured_endpoints(&self) -> Option<&[JsonData]>;
}

// Ok(None) is a team row that does not exist; a team whose metadata column is null is Some(JsonData::Null)
#[async_trait]
pub trait TeamLookup: Send + Sync {
    async fn find_team_metadata(&self, team_id: &str) -> Result<Option<JsonData>, TeamQueryError>;
}
