use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::auxiliary_repository::bindings::MCPServerTable;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableError,
};
use betterlite_db::postgres_store::errors::backend;
use betterlite_mcp::server_record::MCPServer;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use serde::Deserialize;
use thiserror::Error;

const SERVER_ROWS_SQL: &str = r#"SELECT to_jsonb(t)::text FROM (SELECT server_id, alias, server_name FROM "LiteLLM_MCPServerTable") t WHERE t.server_id = ANY($1::text[]) OR t.alias = ANY($1::text[]) OR t.server_name = ANY($1::text[])"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServerNames<'names> {
    pub server_id: Option<&'names str>,
    pub alias: Option<&'names str>,
    pub server_name: Option<&'names str>,
    pub name: Option<&'names str>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct ServerRow {
    pub server_id: String,
    pub alias: Option<String>,
    pub server_name: Option<String>,
}

#[derive(Debug, Error)]
pub enum ResolveError {
    #[error(transparent)]
    Database(#[from] TableError),
}

#[must_use]
pub fn identifier_matches(names: &ServerNames<'_>, identifier: &str) -> bool {
    let candidates = [names.server_id, names.alias, names.server_name, names.name];
    candidates
        .into_iter()
        .flatten()
        .any(|candidate| candidate == identifier)
}

pub async fn db_servers_by_identifiers(
    identifiers: &BTreeSet<String>,
    client: Option<PrismaClient<'_>>,
) -> Result<Vec<ServerRow>, ResolveError> {
    if identifiers.is_empty() {
        return Ok(Vec::new());
    }
    let Some(prisma) = client else {
        return Ok(Vec::new());
    };
    let repository = PrismaTableRepository::new(
        Some(prisma),
        redis_config_change_publisher(None),
        MCPServerTable,
    );
    let access = repository.access().map_err(TableError::Database)?;
    let session = access.read.session().await.map_err(TableError::Database)?;
    let wanted: Vec<&str> = identifiers.iter().map(String::as_str).collect();
    let rows = session
        .query(SERVER_ROWS_SQL, &[&wanted])
        .await
        .map_err(|source| TableError::Database(backend(source)))?;
    let mut found = Vec::with_capacity(rows.len());
    for row in &rows {
        let text: &str = row
            .try_get(0)
            .map_err(|source| TableError::Database(backend(source)))?;
        found.push(serde_json::from_str(text).map_err(TableError::Row)?);
    }
    Ok(found)
}

pub async fn resolve_identifiers_to_ids(
    identifiers: &BTreeSet<String>,
    client: Option<PrismaClient<'_>>,
    registry: &[&MCPServer],
) -> Result<BTreeMap<String, BTreeSet<String>>, ResolveError> {
    let mut resolved: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    if identifiers.is_empty() {
        return Ok(resolved);
    }
    for identifier in identifiers {
        resolved.insert(identifier.clone(), BTreeSet::new());
    }
    for row in db_servers_by_identifiers(identifiers, client).await? {
        if row.server_id.is_empty() {
            continue;
        }
        // The MCP table has no name column, so a stored row matches three of the four attributes
        let names = ServerNames {
            server_id: Some(&row.server_id),
            alias: row.alias.as_deref(),
            server_name: row.server_name.as_deref(),
            name: None,
        };
        add_matches(&mut resolved, identifiers, &names, &row.server_id);
    }
    for server in registry {
        let fields = &server.fields;
        if fields.server_id.is_empty() {
            continue;
        }
        let names = ServerNames {
            server_id: Some(&fields.server_id),
            alias: fields.alias.as_deref(),
            server_name: fields.server_name.as_deref(),
            name: Some(&fields.name),
        };
        add_matches(&mut resolved, identifiers, &names, &fields.server_id);
    }
    Ok(resolved)
}

fn add_matches(
    resolved: &mut BTreeMap<String, BTreeSet<String>>,
    identifiers: &BTreeSet<String>,
    names: &ServerNames<'_>,
    server_id: &str,
) {
    for identifier in identifiers {
        if identifier_matches(names, identifier)
            && let Some(ids) = resolved.get_mut(identifier)
        {
            ids.insert(server_id.to_owned());
        }
    }
}

mod tests;
