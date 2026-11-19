use serde::Deserialize;
use thiserror::Error;
use tokio_postgres::Client;

mod insert;
pub mod model;
mod validate;

mod tests;

use model::{ScimKeyRow, ScimTeamRow, ScimUserRow};

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScimSeed {
    #[serde(default)]
    pub users: Vec<ScimUserRow>,
    #[serde(default)]
    pub teams: Vec<ScimTeamRow>,
    #[serde(default)]
    pub keys: Vec<ScimKeyRow>,
}

#[derive(Debug, Error)]
pub enum ScimFixtureError {
    #[error("scim user row {row} references missing user {user_id}")]
    MissingUser { row: usize, user_id: String },
    #[error("scim {kind} row {row} repeats an earlier {kind}")]
    Duplicate { row: usize, kind: &'static str },
    #[error(
        "scim user row {row} user_role disagrees with the role its key rows give user {user_id}"
    )]
    ConflictingUserRole { row: usize, user_id: String },
    #[error("scim team row {row} references missing team {team_id}")]
    MissingTeam { row: usize, team_id: String },
    #[error("scim key row {row} references no fixture key")]
    UnknownKey { row: usize },
    #[error("scim fixture payload could not be serialized")]
    PayloadNotSerializable {
        #[source]
        source: serde_json::Error,
    },
    #[error("scim fixture database operation failed: {0}")]
    Database(#[source] tokio_postgres::Error),
}

pub async fn seed(client: &Client, scim: &ScimSeed) -> Result<(), ScimFixtureError> {
    insert::users(client, &scim.users).await?;
    insert::teams(client, &scim.teams).await?;
    insert::keys(client, &scim.keys).await
}
