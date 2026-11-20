use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::rows::RowCodec as _;
use betterlite_db::postgres_store::session::DbSession;
use deadpool_postgres::Client;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use crate::agent_rows::AGENT_SELECT_SQL;
use crate::agent_rows::codec::AgentRowCodec;
use crate::agent_rows::codec::StoredAgent;
use crate::object_permissions::prepare::PreparedUpsert;

use super::error::AgentPersistenceError;
use super::error::write_error;
use super::write_plan::AgentColumns;
use super::write_plan::AgentWritePlan;

pub(super) const AGENTS_TABLE: &str = "LiteLLM_AgentsTable";

#[derive(Default)]
pub(super) struct ColumnList<'values> {
    names: Vec<&'static str>,
    placeholders: Vec<String>,
    pub(super) parameters: Vec<&'values (dyn ToSql + Sync)>,
}

impl<'values> ColumnList<'values> {
    pub(super) fn push<T>(&mut self, name: &'static str, value: &'values T, cast: &str)
    where
        T: ToSql + Sync + 'values,
    {
        let position = self.parameters.len().saturating_add(1);
        self.names.push(name);
        self.placeholders.push(format!("${position}{cast}"));
        self.parameters.push(value);
    }

    pub(super) fn insert_sql(&self, table: &str) -> String {
        format!(
            "INSERT INTO \"{table}\" ({}) VALUES ({})",
            self.quoted_names().join(", "),
            self.placeholders.join(", "),
        )
    }

    pub(super) fn update_sql(&self, table: &str, key: &str) -> String {
        let mut assignments = self.assignments_from(1);
        if assignments.is_empty() {
            assignments.push(format!("\"{key}\" = \"{key}\""));
        }
        let set = assignments.join(", ");
        format!("UPDATE \"{table}\" SET {set} WHERE \"{key}\" = $1")
    }

    pub(super) fn quoted_names(&self) -> Vec<String> {
        self.names
            .iter()
            .map(|name| format!("\"{name}\""))
            .collect()
    }

    pub(super) fn assignments_from(&self, start: usize) -> Vec<String> {
        let mut assignments = Vec::new();
        let names = self.names.iter().skip(start);
        let placeholders = self.placeholders.iter().skip(start);
        for (name, placeholder) in names.zip(placeholders) {
            assignments.push(format!("\"{name}\" = {placeholder}"));
        }
        assignments
    }
}

pub(super) fn push_present<'values, T>(
    columns: &mut ColumnList<'values>,
    name: &'static str,
    value: Option<&'values T>,
    cast: &str,
) where
    T: ToSql + Sync + 'values,
{
    if let Some(present) = value {
        columns.push(name, present, cast);
    }
}

fn push_agent_columns<'values>(list: &mut ColumnList<'values>, columns: &'values AgentColumns) {
    push_columns!(
        list,
        columns,
        "agent_name" => agent_name : "",
        "litellm_params" => litellm_params : "::text::jsonb",
        "agent_card_params" => agent_card_params : "::text::jsonb",
        "static_headers" => static_headers : "::text::jsonb",
        "kill_switch" => kill_switch : "::text::jsonb",
        "extra_headers" => extra_headers : "",
        "access_group_ids" => access_group_ids : "",
        "tpm_limit" => tpm_limit : "::bigint",
        "rpm_limit" => rpm_limit : "::bigint",
        "session_tpm_limit" => session_tpm_limit : "::bigint",
        "session_rpm_limit" => session_rpm_limit : "::bigint",
        "enabled" => enabled : "",
        "execution_mode" => execution_mode : "",
        "identity_managed" => identity_managed : "",
        "created_by" => created_by : "",
        "created_at" => created_at : "",
        "updated_by" => updated_by : "",
        "updated_at" => updated_at : "",
    );
}

pub(super) async fn execute(
    tx: &DbSession<'_>,
    sql: &str,
    parameters: &[&(dyn ToSql + Sync)],
) -> Result<u64, AgentPersistenceError> {
    tx.execute(sql, parameters).await.map_err(write_error)
}

fn decode_first(rows: &[Row]) -> Result<Option<StoredAgent>, AgentPersistenceError> {
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    AgentRowCodec
        .decode(row)
        .map_err(|source| AgentPersistenceError::Database(DbError::InvalidRecord { source }))
}

pub(super) enum AgentSource<'query, 'session> {
    Session(&'query DbSession<'session>),
    Client(&'query Client),
}

pub(super) async fn read_agent(
    source: AgentSource<'_, '_>,
    agent_id: &str,
) -> Result<Option<StoredAgent>, AgentPersistenceError> {
    let statement = format!("{AGENT_SELECT_SQL} WHERE a.agent_id = $1");
    let rows = match source {
        AgentSource::Session(tx) => tx.query(&statement, &[&agent_id]).await,
        AgentSource::Client(client) => client.query(&statement, &[&agent_id]).await,
    }
    .map_err(|source| AgentPersistenceError::Database(backend(source)))?;
    decode_first(&rows)
}

pub(super) async fn write_rows(
    tx: &DbSession<'_>,
    request: &RowWrite<'_>,
) -> Result<u64, AgentPersistenceError> {
    if let Some(prepared) = request.permission {
        super::permission_write::write_permission(tx, prepared).await?;
    }
    let agent_id = String::from(request.agent_id);
    let linked = request.link_permission.map(String::from);
    let mut list = ColumnList::default();
    list.push("agent_id", &agent_id, "");
    if let Some(permission) = &linked {
        list.push("object_permission_id", permission, "");
    }
    push_agent_columns(&mut list, &request.plan.columns);
    let sql = if request.insert {
        list.insert_sql(AGENTS_TABLE)
    } else {
        list.update_sql(AGENTS_TABLE, "agent_id")
    };
    let affected = execute(tx, &sql, &list.parameters).await?;
    super::identity_write::write_history(
        tx,
        request.plan.history.as_ref(),
        &agent_id,
        request.history,
    )
    .await?;
    if let Some(identity) = &request.plan.identity {
        super::identity_write::write_identity(tx, &agent_id, identity).await?;
    }
    Ok(affected)
}

pub(super) struct RowWrite<'plan> {
    pub plan: &'plan AgentWritePlan,
    pub agent_id: &'plan str,
    pub permission: Option<&'plan PreparedUpsert>,
    pub link_permission: Option<&'plan str>,
    pub history: bool,
    pub insert: bool,
}
