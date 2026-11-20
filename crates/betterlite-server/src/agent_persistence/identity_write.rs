use betterlite_auth::agent_identity::write_fields::IdentityFields;
use betterlite_auth::agent_identity::write_fields::IdentityHistoryWrite;
use betterlite_auth::agent_identity::write_fields::IdentityRelationWrite;
use betterlite_auth::agent_identity::write_fields::IdentityUpsert;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::DbSession;
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::NaiveDateTime;
use uuid::Uuid;

use super::error::APPLICATION_CONFLICT;
use super::error::AgentPersistenceError;
use super::error::open_failure;
use super::row::ColumnList;
use super::row::execute;
use super::row::push_present;

const IDENTITY_TABLE: &str = "LiteLLM_AgentIdentity";
const HISTORY_TABLE: &str = "LiteLLM_RetiredAgentIdentity";

const HISTORY_LOOKUP_SQL: &str = r#"SELECT agent_id FROM "LiteLLM_RetiredAgentIdentity" WHERE provider = $1 AND tenant_id = $2 AND client_id = $3"#;

fn timestamp_of(fields: &IdentityFields) -> Option<Option<NaiveDateTime>> {
    fields
        .last_authenticated_at
        .map(|stamp| stamp.map(|instant| instant.naive_utc()))
}

fn push_identity_fields<'values>(
    columns: &mut ColumnList<'values>,
    fields: &'values IdentityFields,
    stamp: Option<&'values Option<NaiveDateTime>>,
) {
    push_present(columns, "provider", fields.provider.as_ref(), "");
    push_present(columns, "tenant_id", fields.tenant_id.as_ref(), "");
    push_present(columns, "client_id", fields.client_id.as_ref(), "");
    push_present(columns, "issuer", fields.issuer.as_ref(), "");
    push_present(
        columns,
        "service_principal_id",
        fields.service_principal_id.as_ref(),
        "",
    );
    push_present(
        columns,
        "required_roles",
        fields.required_roles.as_ref(),
        "",
    );
    push_present(
        columns,
        "required_scopes",
        fields.required_scopes.as_ref(),
        "",
    );
    push_present(columns, "active", fields.active.as_ref(), "");
    push_present(columns, "revision", fields.revision.as_ref(), "");
    if let Some(stamp) = stamp {
        columns.push("last_authenticated_at", stamp, "");
    }
}

fn push_conflict_fields<'values>(
    columns: &mut ColumnList<'values>,
    assignments: &mut Vec<String>,
    fields: &'values IdentityFields,
    stamp: Option<&'values Option<NaiveDateTime>>,
) {
    if let Some(value) = &fields.provider {
        assignments.push(columns.push_conflict("provider", value, ""));
    }
    if let Some(value) = &fields.tenant_id {
        assignments.push(columns.push_conflict("tenant_id", value, ""));
    }
    if let Some(value) = &fields.client_id {
        assignments.push(columns.push_conflict("client_id", value, ""));
    }
    if let Some(value) = &fields.issuer {
        assignments.push(columns.push_conflict("issuer", value, ""));
    }
    if let Some(value) = &fields.service_principal_id {
        assignments.push(columns.push_conflict("service_principal_id", value, ""));
    }
    if let Some(value) = &fields.required_roles {
        assignments.push(columns.push_conflict("required_roles", value, ""));
    }
    if let Some(value) = &fields.required_scopes {
        assignments.push(columns.push_conflict("required_scopes", value, ""));
    }
    if let Some(value) = &fields.active {
        assignments.push(columns.push_conflict("active", value, ""));
    }
    if let Some(value) = &fields.revision {
        assignments.push(columns.push_conflict("revision", value, ""));
    }
    if let Some(stamp) = stamp {
        assignments.push(columns.push_conflict("last_authenticated_at", stamp, ""));
    }
}

// A prior history row for another agent is the second conflict text; the same agent is not inserted twice.
pub(super) async fn history_should_insert(
    store: &PostgresStore,
    history: Option<&IdentityHistoryWrite>,
    existing_agent_id: Option<&str>,
) -> Result<bool, AgentPersistenceError> {
    let Some(history) = history else {
        return Ok(false);
    };
    let key = &history.create.key;
    let client = store.session().await.map_err(open_failure)?;
    let row = client
        .query_opt(
            HISTORY_LOOKUP_SQL,
            &[&key.provider, &key.tenant_id, &key.client_id],
        )
        .await
        .map_err(|source| AgentPersistenceError::Database(backend(source)))?;
    let Some(row) = row else {
        return Ok(true);
    };
    let prior: Option<String> = row
        .try_get("agent_id")
        .map_err(|source| AgentPersistenceError::Database(backend(source)))?;
    if existing_agent_id.is_some() && prior.as_deref() == existing_agent_id {
        return Ok(false);
    }
    Err(AgentPersistenceError::Conflict(APPLICATION_CONFLICT))
}

pub(super) async fn write_history(
    tx: &DbSession<'_>,
    history: Option<&IdentityHistoryWrite>,
    agent_id: &str,
    insert: bool,
) -> Result<(), AgentPersistenceError> {
    let Some(history) = history else {
        return Ok(());
    };
    if !insert {
        return Ok(());
    }
    let binding_id = Uuid::new_v4().to_string();
    let agent_key = String::from(agent_id);
    let entry = &history.create;
    let mut list = ColumnList::default();
    list.push("binding_id", &binding_id, "");
    list.push("agent_id", &agent_key, "");
    list.push("provider", &entry.key.provider, "");
    list.push("issuer", &entry.issuer, "");
    list.push("tenant_id", &entry.key.tenant_id, "");
    list.push("client_id", &entry.key.client_id, "");
    let sql = list.insert_sql(HISTORY_TABLE);
    execute(tx, &sql, &list.parameters).await?;
    Ok(())
}

pub(super) async fn write_identity(
    tx: &DbSession<'_>,
    agent_id: &str,
    relation: &IdentityRelationWrite,
) -> Result<(), AgentPersistenceError> {
    match relation {
        IdentityRelationWrite::Create(fields) => insert_identity(tx, agent_id, fields).await,
        IdentityRelationWrite::Update(fields) => update_identity(tx, agent_id, fields).await,
        IdentityRelationWrite::Upsert(upsert) => upsert_identity(tx, agent_id, upsert).await,
    }
}

async fn insert_identity(
    tx: &DbSession<'_>,
    agent_id: &str,
    fields: &IdentityFields,
) -> Result<(), AgentPersistenceError> {
    let agent_key = String::from(agent_id);
    let stamp = timestamp_of(fields);
    let mut list = ColumnList::default();
    list.push("agent_id", &agent_key, "");
    push_identity_fields(&mut list, fields, stamp.as_ref());
    let sql = list.insert_sql(IDENTITY_TABLE);
    execute(tx, &sql, &list.parameters).await?;
    Ok(())
}

async fn update_identity(
    tx: &DbSession<'_>,
    agent_id: &str,
    fields: &IdentityFields,
) -> Result<(), AgentPersistenceError> {
    let agent_key = String::from(agent_id);
    let stamp = timestamp_of(fields);
    let mut list = ColumnList::default();
    list.push("agent_id", &agent_key, "");
    push_identity_fields(&mut list, fields, stamp.as_ref());
    let sql = list.update_sql(IDENTITY_TABLE, "agent_id");
    execute(tx, &sql, &list.parameters).await?;
    Ok(())
}

async fn upsert_identity(
    tx: &DbSession<'_>,
    agent_id: &str,
    upsert: &IdentityUpsert,
) -> Result<(), AgentPersistenceError> {
    let agent_key = String::from(agent_id);
    let create_stamp = timestamp_of(&upsert.create);
    let update_stamp = timestamp_of(&upsert.update);
    let mut list = ColumnList::default();
    list.push("agent_id", &agent_key, "");
    push_identity_fields(&mut list, &upsert.create, create_stamp.as_ref());
    let insert = list.insert_sql(IDENTITY_TABLE);
    let mut assignments = Vec::new();
    push_conflict_fields(
        &mut list,
        &mut assignments,
        &upsert.update,
        update_stamp.as_ref(),
    );
    if assignments.is_empty() {
        return Ok(());
    }
    let sql = format!(
        "{insert} ON CONFLICT (\"agent_id\") DO UPDATE SET {}",
        assignments.join(", "),
    );
    execute(tx, &sql, &list.parameters).await?;
    Ok(())
}

mod tests;
