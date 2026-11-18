use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::resource_rows::project::ProjectTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use crate::auth_cache::payload::{self, PayloadError};

// the five columns the prefetch SELECT returns, named as LiteLLM's _RowKind
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub enum RowKind {
    #[serde(rename = "user_row")]
    User,
    #[serde(rename = "team_row")]
    Team,
    #[serde(rename = "membership_row")]
    Membership,
    #[serde(rename = "organization_row")]
    Organization,
    #[serde(rename = "project_row")]
    Project,
}

impl RowKind {
    // the column name and the model type it validates as, for the redacted diagnostic
    #[must_use]
    pub const fn names(self) -> (&'static str, &'static str) {
        match self {
            Self::User => ("user_row", "LiteLLM_UserTable"),
            Self::Team => ("team_row", "LiteLLM_TeamTableCachedObj"),
            Self::Membership => ("membership_row", "LiteLLM_TeamMembership"),
            Self::Organization => ("organization_row", "LiteLLM_OrganizationTable"),
            Self::Project => ("project_row", "LiteLLM_ProjectTableCachedObj"),
        }
    }
}

// the query returns one nullable row per kind, keyed by that kind
pub type RowValues = BTreeMap<RowKind, JsonData>;

// LiteLLM's LiteLLM_ProjectTableCachedObj, which the shared row model does not carry
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CachedProject {
    #[serde(flatten)]
    pub project: ProjectTable,
    #[serde(default)]
    pub last_refreshed_at: Option<f64>,
}

// the validated object a row kind writes back; every variant serializes as its model
#[derive(Clone, Debug)]
pub enum RowObject {
    User(LiteLLMUserTable),
    Team(LiteLLMTeamTableCachedObj),
    Membership(LiteLLMTeamMembership),
    Organization(OrganizationTable),
    Project(CachedProject),
}

impl RowObject {
    // CacheCodec.serialize(value, model_type) is the model's json dump
    pub fn payload(&self) -> Result<Json, PayloadError> {
        match self {
            Self::User(row) => payload::encode(row),
            Self::Team(row) => payload::encode(row),
            Self::Membership(row) => payload::encode(row),
            Self::Organization(row) => payload::encode(row),
            Self::Project(row) => payload::encode(row),
        }
    }
}

// one validation per requested kind, stamping the post-query time onto team and project only
#[must_use]
pub fn validate_row(
    kind: RowKind,
    value: Option<&JsonData>,
    refreshed_at: f64,
) -> Option<RowObject> {
    let value = value?;
    match decode(kind, value, refreshed_at) {
        Ok(object) => Some(object),
        Err(error) => {
            let (row, model) = kind.names();
            // the rejected columns stay out of the line: payloads can hold keys and user rows
            tracing::warn!(
                row,
                model,
                category = ?error.classify(),
                "auth prefetch: the row did not validate as its model"
            );
            None
        }
    }
}

// the model's own decoder validates the row; only the timestamp is forced to the query time
fn decode(
    kind: RowKind,
    value: &JsonData,
    refreshed_at: f64,
) -> Result<RowObject, serde_json::Error> {
    let wire = serde_json::to_value(value)?;
    match kind {
        RowKind::User => serde_json::from_value::<LiteLLMUserTable>(wire).map(RowObject::User),
        RowKind::Team => {
            serde_json::from_value::<LiteLLMTeamTableCachedObj>(wire).map(|mut row| {
                row.last_refreshed_at = Some(refreshed_at);
                RowObject::Team(row)
            })
        }
        RowKind::Membership => {
            serde_json::from_value::<LiteLLMTeamMembership>(wire).map(RowObject::Membership)
        }
        RowKind::Organization => {
            serde_json::from_value::<OrganizationTable>(wire).map(RowObject::Organization)
        }
        RowKind::Project => serde_json::from_value::<CachedProject>(wire).map(|mut row| {
            row.last_refreshed_at = Some(refreshed_at);
            RowObject::Project(row)
        }),
    }
}
