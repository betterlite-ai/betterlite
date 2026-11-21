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
use super::error::write_error;
use super::row::ColumnList;
use super::row::push_present;

const IDENTITY_TABLE: &str = "LiteLLM_AgentIdentity";
const HISTORY_TABLE: &str = "LiteLLM_RetiredAgentIdentity";

const HISTORY_LOOKUP_SQL: &str = r#"SELECT agent_id FROM "LiteLLM_RetiredAgentIdentity" WHERE provider = $1 AND tenant_id = $2 AND client_id = $3"#;

fn push_identity_fields<'values>(
    columns: &mut ColumnList<'values>,
    fields: &'values IdentityFields,
    stamp: Option<&'values Option<NaiveDateTime>>,
) {
    push_columns!(
        columns,
        fields,
        "provider" => provider : "",
        "tenant_id" => tenant_id : "",
        "client_id" => client_id : "",
        "issuer" => issuer : "",
        "service_principal_id" => service_principal_id : "",
        "required_roles" => required_roles : "",
        "required_scopes" => required_scopes : "",
        "active" => active : "",
        "revision" => revision : "",
    );
    if let Some(at) = stamp {
        columns.push("last_authenticated_at", at, "");
    }
}

// A prior history row for another agent is the second conflict text; the same agent is not inserted twice.
pub(super) async fn history_should_insert(
    store: &PostgresStore,
    history: Option<&IdentityHistoryWrite>,
    existing_agent_id: Option<&str>,
) -> Result<bool, AgentPersistenceError> {
    let Some(pending) = history else {
        return Ok(false);
    };
    let key = &pending.create.key;
    let client = store.session().await.map_err(open_failure)?;
    let queried = client
        .query_opt(
            HISTORY_LOOKUP_SQL,
            &[&key.provider, &key.tenant_id, &key.client_id],
        )
        .await
        .map_err(|source| AgentPersistenceError::Database(backend(source)))?;
    let Some(row) = queried else {
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
    let Some(pending) = history else {
        return Ok(());
    };
    if !insert {
        return Ok(());
    }
    let binding_id = Uuid::new_v4().to_string();
    let agent_key = String::from(agent_id);
    let entry = &pending.create;
    let mut list = ColumnList::default();
    list.push("binding_id", &binding_id, "");
    list.push("agent_id", &agent_key, "");
    list.push("provider", &entry.key.provider, "");
    list.push("issuer", &entry.issuer, "");
    list.push("tenant_id", &entry.key.tenant_id, "");
    list.push("client_id", &entry.key.client_id, "");
    let sql = list.insert_sql(HISTORY_TABLE);
    tx.execute(&sql, &list.parameters)
        .await
        .map_err(write_error)?;
    Ok(())
}

pub(super) async fn write_identity(
    tx: &DbSession<'_>,
    agent_id: &str,
    relation: &IdentityRelationWrite,
) -> Result<(), AgentPersistenceError> {
    match relation {
        IdentityRelationWrite::Create(fields) => {
            write_identity_row(tx, agent_id, fields, true).await
        }
        IdentityRelationWrite::Update(fields) => {
            write_identity_row(tx, agent_id, fields, false).await
        }
        IdentityRelationWrite::Upsert(upsert) => upsert_identity(tx, agent_id, upsert).await,
    }
}

async fn write_identity_row(
    tx: &DbSession<'_>,
    agent_id: &str,
    fields: &IdentityFields,
    insert: bool,
) -> Result<(), AgentPersistenceError> {
    let agent_key = String::from(agent_id);
    let stamp = fields
        .last_authenticated_at
        .map(|value| value.map(|instant| instant.naive_utc()));
    let mut list = ColumnList::default();
    list.push("agent_id", &agent_key, "");
    push_identity_fields(&mut list, fields, stamp.as_ref());
    let sql = if insert {
        list.insert_sql(IDENTITY_TABLE)
    } else {
        list.update_sql(IDENTITY_TABLE, "agent_id")
    };
    tx.execute(&sql, &list.parameters)
        .await
        .map_err(write_error)?;
    Ok(())
}

async fn upsert_identity(
    tx: &DbSession<'_>,
    agent_id: &str,
    upsert: &IdentityUpsert,
) -> Result<(), AgentPersistenceError> {
    let agent_key = String::from(agent_id);
    let create_stamp = upsert
        .create
        .last_authenticated_at
        .map(|value| value.map(|instant| instant.naive_utc()));
    let update_stamp = upsert
        .update
        .last_authenticated_at
        .map(|value| value.map(|instant| instant.naive_utc()));
    let mut list = ColumnList::default();
    list.push("agent_id", &agent_key, "");
    push_identity_fields(&mut list, &upsert.create, create_stamp.as_ref());
    let insert = list.insert_sql(IDENTITY_TABLE);
    let start = list.parameters.len();
    push_identity_fields(&mut list, &upsert.update, update_stamp.as_ref());
    let assignments = list.assignments_from(start);
    if assignments.is_empty() {
        return Ok(());
    }
    let sql = format!(
        "{insert} ON CONFLICT (\"agent_id\") DO UPDATE SET {}",
        assignments.join(", "),
    );
    tx.execute(&sql, &list.parameters)
        .await
        .map_err(write_error)?;
    Ok(())
}

mod tests;
