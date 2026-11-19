use std::collections::BTreeMap;
use std::result::Result as StdResult;

use secrecy::ExposeSecret as _;
use tokio_postgres::Client;
use tokio_postgres::types::ToSql;

use super::ScimFixtureError;
use super::model::{ScimKeyRow, ScimTeamRow, ScimUserRow};
use crate::dashboard::payload::JsonPayload;
use crate::seed::{CallerRole, FixtureSeed};

type FixtureResult<T> = StdResult<T, ScimFixtureError>;

// every column is optional on the row, so COALESCE keeps what the base seed inserted
const USER_UPDATE: &str = "UPDATE \"LiteLLM_UserTable\" SET \"created_at\" = COALESCE($2::text::timestamp(3), \"created_at\"), \"updated_at\" = COALESCE($3::text::timestamp(3), \"updated_at\"), \"user_alias\" = COALESCE($4, \"user_alias\"), \"sso_user_id\" = COALESCE($5, \"sso_user_id\"), \"user_role\" = COALESCE($6, \"user_role\"), \"teams\" = COALESCE($7, \"teams\"), \"metadata\" = COALESCE($8::text::jsonb, \"metadata\") WHERE \"user_id\" = $1";

// LiteLLM writes members_with_roles as a JSON array, while the baseline column defaults to '{}'
const TEAM_UPDATE: &str = "UPDATE \"LiteLLM_TeamTable\" SET \"created_at\" = COALESCE($2::text::timestamp(3), \"created_at\"), \"updated_at\" = COALESCE($3::text::timestamp(3), \"updated_at\"), \"members_with_roles\" = $4::text::jsonb, \"members\" = COALESCE($5, \"members\"), \"metadata\" = COALESCE($6::text::jsonb, \"metadata\") WHERE \"team_id\" = $1";

// blocked stays nullable because LiteLLM treats NULL and false as two different unblocked states
const KEY_UPDATE: &str = "UPDATE \"LiteLLM_VerificationToken\" SET \"blocked\" = COALESCE($2, \"blocked\"), \"metadata\" = COALESCE($3::text::jsonb, \"metadata\") WHERE \"token\" = $1";

async fn execute(client: &Client, sql: &str, values: &[&(dyn ToSql + Sync)]) -> FixtureResult<()> {
    client
        .execute(sql, values)
        .await
        .map_err(ScimFixtureError::Database)?;
    Ok(())
}

fn metadata_text(
    metadata: Option<&BTreeMap<String, JsonPayload>>,
) -> FixtureResult<Option<String>> {
    metadata
        .map(serde_json::to_string)
        .transpose()
        .map_err(|source| ScimFixtureError::PayloadNotSerializable { source })
}

async fn execute_user(client: &Client, row: &ScimUserRow) -> FixtureResult<()> {
    let created_at = row
        .created_at
        .map(|moment| moment.format("%Y-%m-%dT%H:%M:%S%.f").to_string());
    let updated_at = row
        .updated_at
        .map(|moment| moment.format("%Y-%m-%dT%H:%M:%S%.f").to_string());
    let user_role = row.user_role.map(CallerRole::as_wire);
    let metadata = metadata_text(row.metadata.as_ref())?;
    execute(
        client,
        USER_UPDATE,
        &[
            &row.user_id,
            &created_at,
            &updated_at,
            &row.user_alias,
            &row.sso_user_id,
            &user_role,
            &row.teams,
            &metadata,
        ],
    )
    .await
}

async fn execute_team(client: &Client, row: &ScimTeamRow) -> FixtureResult<()> {
    let created_at = row
        .created_at
        .map(|moment| moment.format("%Y-%m-%dT%H:%M:%S%.f").to_string());
    let updated_at = row
        .updated_at
        .map(|moment| moment.format("%Y-%m-%dT%H:%M:%S%.f").to_string());
    let members_with_roles = serde_json::to_string(&row.members_with_roles)
        .map_err(|source| ScimFixtureError::PayloadNotSerializable { source })?;
    let metadata = metadata_text(row.metadata.as_ref())?;
    let mut values: Vec<&(dyn ToSql + Sync)> = Vec::with_capacity(6);
    values.push(&row.team_id);
    values.push(&created_at);
    values.push(&updated_at);
    values.push(&members_with_roles);
    values.push(&row.members);
    values.push(&metadata);
    execute(client, TEAM_UPDATE, &values).await
}

async fn execute_key(client: &Client, row: &ScimKeyRow) -> FixtureResult<()> {
    let token = FixtureSeed::key_hash(row.token.expose_secret());
    let metadata = metadata_text(row.metadata.as_ref())?;
    execute(client, KEY_UPDATE, &[&token, &row.blocked, &metadata]).await
}

pub(super) async fn users(client: &Client, rows: &[ScimUserRow]) -> FixtureResult<()> {
    for row in rows {
        execute_user(client, row).await?;
    }
    Ok(())
}

pub(super) async fn teams(client: &Client, rows: &[ScimTeamRow]) -> FixtureResult<()> {
    for row in rows {
        execute_team(client, row).await?;
    }
    Ok(())
}

pub(super) async fn keys(client: &Client, rows: &[ScimKeyRow]) -> FixtureResult<()> {
    for row in rows {
        execute_key(client, row).await?;
    }
    Ok(())
}
