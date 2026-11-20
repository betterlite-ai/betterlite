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

    // Call after insert_sql: the conflict columns join the parameter list but not the insert list.
    pub(super) fn push_conflict<T>(
        &mut self,
        name: &'static str,
        value: &'values T,
        cast: &str,
    ) -> String
    where
        T: ToSql + Sync + 'values,
    {
        self.push(name, value, cast);
        let placeholder = self.placeholders.last().cloned().unwrap_or_default();
        format!("\"{name}\" = {placeholder}")
    }

    pub(super) fn insert_sql(&self, table: &str) -> String {
        format!(
            "INSERT INTO \"{table}\" ({}) VALUES ({})",
            self.quoted_names().join(", "),
            self.placeholders.join(", "),
        )
    }

    pub(super) fn update_sql(&self, table: &str, key: &str) -> String {
        let mut assignments: Vec<String> = self
            .names
            .iter()
            .zip(&self.placeholders)
            .skip(1)
            .map(|(name, placeholder)| format!("\"{name}\" = {placeholder}"))
            .collect();
        if assignments.is_empty() {
            assignments.push(format!("\"{key}\" = \"{key}\""));
        }
        format!(
            "UPDATE \"{table}\" SET {} WHERE \"{key}\" = $1",
            assignments.join(", "),
        )
    }

    pub(super) fn quoted_names(&self) -> Vec<String> {
        self.names
            .iter()
            .map(|name| format!("\"{name}\""))
            .collect()
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

fn push_json_columns<'values>(list: &mut ColumnList<'values>, columns: &'values AgentColumns) {
    push_present(list, "agent_name", columns.agent_name.as_ref(), "");
    push_present(
        list,
        "litellm_params",
        columns.litellm_params.as_ref(),
        "::text::jsonb",
    );
    push_present(
        list,
        "agent_card_params",
        columns.agent_card_params.as_ref(),
        "::text::jsonb",
    );
    push_present(
        list,
        "static_headers",
        columns.static_headers.as_ref(),
        "::text::jsonb",
    );
    push_present(
        list,
        "kill_switch",
        columns.kill_switch.as_ref(),
        "::text::jsonb",
    );
}

fn push_list_columns<'values>(list: &mut ColumnList<'values>, columns: &'values AgentColumns) {
    push_present(list, "extra_headers", columns.extra_headers.as_ref(), "");
    push_present(
        list,
        "access_group_ids",
        columns.access_group_ids.as_ref(),
        "",
    );
}

fn push_limit_columns<'values>(list: &mut ColumnList<'values>, columns: &'values AgentColumns) {
    push_present(list, "tpm_limit", columns.tpm_limit.as_ref(), "::bigint");
    push_present(list, "rpm_limit", columns.rpm_limit.as_ref(), "::bigint");
    push_present(
        list,
        "session_tpm_limit",
        columns.session_tpm_limit.as_ref(),
        "::bigint",
    );
    push_present(
        list,
        "session_rpm_limit",
        columns.session_rpm_limit.as_ref(),
        "::bigint",
    );
}

fn push_meta_columns<'values>(list: &mut ColumnList<'values>, columns: &'values AgentColumns) {
    push_present(list, "enabled", columns.enabled.as_ref(), "");
    push_present(list, "execution_mode", columns.execution_mode.as_ref(), "");
    push_present(
        list,
        "identity_managed",
        columns.identity_managed.as_ref(),
        "",
    );
    push_present(list, "created_by", columns.created_by.as_ref(), "");
    push_present(list, "created_at", columns.created_at.as_ref(), "");
    push_present(list, "updated_by", columns.updated_by.as_ref(), "");
    push_present(list, "updated_at", columns.updated_at.as_ref(), "");
}

fn push_agent_columns<'values>(list: &mut ColumnList<'values>, columns: &'values AgentColumns) {
    push_json_columns(list, columns);
    push_list_columns(list, columns);
    push_limit_columns(list, columns);
    push_meta_columns(list, columns);
}

pub(super) async fn execute(
    tx: &DbSession<'_>,
    sql: &str,
    parameters: &[&(dyn ToSql + Sync)],
) -> Result<u64, AgentPersistenceError> {
    match tx.execute(sql, parameters).await {
        Ok(affected) => Ok(affected),
        Err(source) => Err(write_error(source)),
    }
}

fn decode_first(rows: &[Row]) -> Result<Option<StoredAgent>, AgentPersistenceError> {
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    AgentRowCodec
        .decode(row)
        .map_err(|source| AgentPersistenceError::Database(DbError::InvalidRecord { source }))
}

pub(super) async fn read_agent(
    tx: &DbSession<'_>,
    agent_id: &str,
) -> Result<Option<StoredAgent>, AgentPersistenceError> {
    let statement = format!("{AGENT_SELECT_SQL} WHERE a.agent_id = $1");
    let rows = tx
        .query(&statement, &[&agent_id])
        .await
        .map_err(|source| AgentPersistenceError::Database(backend(source)))?;
    decode_first(&rows)
}

pub(super) async fn read_agent_on(
    client: &Client,
    agent_id: &str,
) -> Result<Option<StoredAgent>, AgentPersistenceError> {
    let statement = format!("{AGENT_SELECT_SQL} WHERE a.agent_id = $1");
    let rows = client
        .query(&statement, &[&agent_id])
        .await
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
