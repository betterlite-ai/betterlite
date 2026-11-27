use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_repository::decoder::{TeamRecord, TeamRecordError, decode_team_record};
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use deadpool_postgres::Client;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use crate::team_fields::tables::{raw_team_table, team_table};

use super::TeamListQueryError;

// _raw_team_db(TeamRepository).find_many(include={"litellm_model_table": True})
pub(super) const ORG_TEAMS_SQL: &str = "SELECT row_to_json(x)::text FROM (SELECT t.*, to_jsonb(m.*) AS litellm_model_table FROM \"LiteLLM_TeamTable\" t LEFT JOIN \"LiteLLM_ModelTable\" m ON t.\"model_id\" = m.\"id\" WHERE t.\"organization_id\" = ANY($1::text[])) x";
pub(super) const ALL_TEAMS_SQL: &str = "SELECT row_to_json(x)::text FROM (SELECT t.*, to_jsonb(m.*) AS litellm_model_table FROM \"LiteLLM_TeamTable\" t LEFT JOIN \"LiteLLM_ModelTable\" m ON t.\"model_id\" = m.\"id\") x";
// _tokens_db.group_by(by=["team_id"], where={"team_id": {"in": page_team_ids}}, count={"team_id": True})
const TEAM_KEY_COUNTS_SQL: &str = "SELECT team_id, count(team_id)::bigint FROM \"LiteLLM_VerificationToken\" WHERE team_id = ANY($1::text[]) GROUP BY team_id";
// _access_group_db.find_many(where={"access_group_id": {"in": unique_ids}})
const ACCESS_GROUPS_SQL: &str = "SELECT row_to_json(g)::text FROM \"LiteLLM_AccessGroupTable\" g WHERE g.\"access_group_id\" = ANY($1::text[])";

async fn open_session(client: Option<PrismaClient<'_>>) -> Result<Client, TeamListQueryError> {
    let repository = team_table(client)?;
    let store = raw_team_table(&repository).store;
    store.session().await.map_err(TeamListQueryError::from)
}

async fn query_records(
    client: Option<PrismaClient<'_>>,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<TeamRecord>, TeamListQueryError> {
    let session = open_session(client).await?;
    let rows = session.query(sql, params).await.map_err(backend)?;
    rows.iter().map(record_of_row).collect()
}

fn record_of_row(row: &Row) -> Result<TeamRecord, TeamListQueryError> {
    let text: String = row.try_get(0).map_err(backend)?;
    let record = TeamRecord::parse(text.as_bytes())?.ok_or(TeamRecordError::NotAnObject)?;
    Ok(record)
}

pub async fn find_teams(
    client: Option<PrismaClient<'_>>,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<LiteLLMTeamTable>, TeamListQueryError> {
    let records = query_records(client, sql, params).await?;
    let mut teams = Vec::with_capacity(records.len());
    for record in &records {
        teams.push(decode_team_record(Some(record))?.ok_or(TeamRecordError::NotAnObject)?);
    }
    Ok(teams)
}

pub async fn key_counts_by_team(
    client: Option<PrismaClient<'_>>,
    teams: &[TeamRecord],
) -> Result<BTreeMap<String, i64>, TeamListQueryError> {
    let page_team_ids: Vec<String> = teams.iter().filter_map(team_id_of).collect();
    if page_team_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let session = open_session(client).await?;
    let rows = session
        .query(TEAM_KEY_COUNTS_SQL, &[&page_team_ids])
        .await
        .map_err(backend)?;
    let mut counts = BTreeMap::new();
    for row in &rows {
        let team_id: String = row.try_get(0).map_err(backend)?;
        let count: i64 = row.try_get(1).map_err(backend)?;
        counts.insert(team_id, count);
    }
    Ok(counts)
}

fn team_id_of(record: &TeamRecord) -> Option<String> {
    match record.fields.get("team_id") {
        Some(JsonData::String(team_id)) if !team_id.is_empty() => Some(team_id.clone()),
        Some(_) | None => None,
    }
}

pub async fn access_group_resources(
    client: Option<PrismaClient<'_>>,
    access_group_ids: &[String],
) -> Result<BTreeMap<String, AccessGroupTable>, TeamListQueryError> {
    let unique_ids = unique_ids(access_group_ids);
    let Some(connected) = client else {
        return Ok(BTreeMap::new());
    };
    let session = open_session(Some(connected)).await?;
    let rows = session
        .query(ACCESS_GROUPS_SQL, &[&unique_ids])
        .await
        .map_err(backend)?;
    let mut groups = BTreeMap::new();
    for row in &rows {
        let text: String = row.try_get(0).map_err(backend)?;
        let group: AccessGroupTable =
            serde_json::from_str(&text).map_err(TeamListQueryError::AccessGroup)?;
        groups.insert(group.access_group_id.clone(), group);
    }
    Ok(groups)
}

// frozenset(all_access_group_ids) in first-seen order, so the one query binds each ID once
#[must_use]
pub fn unique_ids(ids: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut distinct = Vec::new();
    for id in ids {
        if seen.insert(id.as_str()) {
            distinct.push(id.clone());
        }
    }
    distinct
}
