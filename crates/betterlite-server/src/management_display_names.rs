use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::future::Future;
use std::time::Duration;

use betterlite_auth::agent_registry::{AgentRegistry, AgentRegistryApi as _};
use betterlite_db::auxiliary_repository::bindings::{AgentsTable, MCPServerTable};
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableBinding, TableError,
};
use betterlite_db::database_refresh::handle::{PoolFuture, PoolTransport};
use betterlite_db::database_replica::client::{Attribute, DatabaseClient, DatabaseLifecycle};
use betterlite_db::database_replica::pinned::WriterPinnedClient;
use betterlite_db::key_hash::KeyHash;
use betterlite_db::key_repository::table::TOKEN_TABLE_ATTRIBUTE;
use betterlite_db::key_repository::{TokenRepositoryError, VerificationTokenRepository};
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_mcp::server_record::MCPServer;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use deadpool_postgres::Pool;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use tokio_postgres::Row;

mod tests;

const MCP_SERVER_NAMES_SQL: &str = "SELECT server_id AS id, COALESCE(NULLIF(alias, ''), NULLIF(server_name, '')) AS label FROM \"LiteLLM_MCPServerTable\" WHERE server_id = ANY($1::text[])";
const AGENT_NAMES_SQL: &str = "SELECT agent_id AS id, agent_name AS label FROM \"LiteLLM_AgentsTable\" WHERE agent_id = ANY($1::text[])";

#[derive(Debug, Error)]
pub enum DisplayNameError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Table(#[from] TableError),
    #[error(transparent)]
    Token(#[from] TokenRepositoryError<Infallible>),
}

pub trait DisplayNameStore: Send + Sync {
    fn mcp_server_names(
        &self,
        ids: BTreeSet<String>,
    ) -> impl Future<Output = Result<BTreeMap<String, String>, DisplayNameError>> + Send;

    fn agent_names(
        &self,
        ids: BTreeSet<String>,
    ) -> impl Future<Output = Result<BTreeMap<String, String>, DisplayNameError>> + Send;

    fn key_names(
        &self,
        tokens: BTreeSet<String>,
    ) -> impl Future<Output = Result<BTreeMap<String, String>, DisplayNameError>> + Send;
}

pub async fn mcp_server_display_names<S>(
    store: &S,
    server_ids: &[String],
    config_servers: &BTreeMap<String, MCPServer>,
) -> Result<BTreeMap<String, String>, DisplayNameError>
where
    S: DisplayNameStore + ?Sized,
{
    if server_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let wanted: BTreeSet<String> = server_ids.iter().cloned().collect();
    let mut names = BTreeMap::new();
    for (server_id, server) in config_servers {
        if !wanted.contains(server_id) {
            continue;
        }
        let fields = &server.fields;
        let label = fields
            .alias
            .as_deref()
            .or(fields.server_name.as_deref())
            .unwrap_or(&fields.name);
        names.insert(server_id.clone(), label.to_owned());
    }
    names.extend(store.mcp_server_names(wanted).await?);
    Ok(names)
}

pub async fn agent_display_names<S>(
    store: &S,
    agent_ids: &[String],
    registry: &AgentRegistry,
) -> Result<BTreeMap<String, String>, DisplayNameError>
where
    S: DisplayNameStore + ?Sized,
{
    if agent_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let wanted: BTreeSet<String> = agent_ids.iter().cloned().collect();
    let mut names = BTreeMap::new();
    for agent in registry.get_agent_list(None) {
        for alias_id in registry.ids_for_agent(&agent.agent_id) {
            if wanted.contains(&alias_id) {
                names.insert(alias_id, agent.agent_name.clone());
            }
        }
    }
    names.extend(store.agent_names(wanted).await?);
    Ok(names)
}

pub async fn key_display_names<S>(
    store: &S,
    tokens: &[String],
) -> Result<BTreeMap<String, String>, DisplayNameError>
where
    S: DisplayNameStore + ?Sized,
{
    if tokens.is_empty() {
        return Ok(BTreeMap::new());
    }
    store.key_names(tokens.iter().cloned().collect()).await
}

impl DisplayNameStore for PrismaClient<'_> {
    fn mcp_server_names(
        &self,
        ids: BTreeSet<String>,
    ) -> impl Future<Output = Result<BTreeMap<String, String>, DisplayNameError>> + Send {
        table_names(*self, MCPServerTable, MCP_SERVER_NAMES_SQL, ids)
    }

    fn agent_names(
        &self,
        ids: BTreeSet<String>,
    ) -> impl Future<Output = Result<BTreeMap<String, String>, DisplayNameError>> + Send {
        table_names(*self, AgentsTable, AGENT_NAMES_SQL, ids)
    }

    fn key_names(
        &self,
        tokens: BTreeSet<String>,
    ) -> impl Future<Output = Result<BTreeMap<String, String>, DisplayNameError>> + Send {
        async move {
            let client = PostgresTokenClient { store: self.writer };
            let repository = VerificationTokenRepository {
                database: Some(WriterPinnedClient::Writer(&client)),
            };
            let hashes: Vec<KeyHash> = tokens.into_iter().map(KeyHash::from).collect();
            let mut names = BTreeMap::new();
            for row in repository.find_many_in(hashes).await? {
                let (Some(token), Some(alias)) = (row.token, row.key_alias) else {
                    continue;
                };
                if alias.is_empty() {
                    continue;
                }
                names.insert(token.expose_secret().to_owned(), alias);
            }
            Ok(names)
        }
    }
}

async fn table_names<T>(
    client: PrismaClient<'_>,
    binding: T,
    sql: &'static str,
    ids: BTreeSet<String>,
) -> Result<BTreeMap<String, String>, DisplayNameError>
where
    T: TableBinding,
{
    let repository =
        PrismaTableRepository::new(Some(client), redis_config_change_publisher(None), binding);
    let session = repository.access()?.read.session().await?;
    let requested: Vec<String> = ids.into_iter().collect();
    let rows = session
        .query(sql, &[&requested])
        .await
        .map_err(|source| TableError::Database(backend(source)))?;
    let mut names = BTreeMap::new();
    for row in &rows {
        if let Some((id, found)) = row_name(row)? {
            names.insert(id, found);
        }
    }
    Ok(names)
}

fn row_name(row: &Row) -> Result<Option<(String, String)>, DisplayNameError> {
    let id: String = row
        .try_get("id")
        .map_err(|source| TableError::Database(backend(source)))?;
    let label: Option<String> = row
        .try_get("label")
        .map_err(|source| TableError::Database(backend(source)))?;
    Ok(label.map(|found| (id, found)))
}

// the token repository only reads through attribute(), so these lifecycle steps never run
struct PostgresTokenClient<'db> {
    store: &'db PostgresStore,
}

impl DatabaseLifecycle for PostgresTokenClient<'_> {
    type Error = Infallible;

    #[cfg_attr(test, mutants::skip)]
    fn is_connected(&self) -> bool {
        self.store.pool.is_some()
    }

    #[cfg_attr(test, mutants::skip)]
    fn connect(&self, _timeout: Option<Duration>) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    #[cfg_attr(test, mutants::skip)]
    fn disconnect(&self, _timeout: Option<Duration>) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    #[cfg_attr(test, mutants::skip)]
    fn recreate<'database>(
        &'database self,
        _url: &'database SecretString,
        _transport: Option<&'database PoolTransport>,
        _expected_generation: Option<u64>,
    ) -> PoolFuture<'database, bool, Infallible> {
        Box::pin(async { Ok(false) })
    }

    #[cfg_attr(test, mutants::skip)]
    fn start_refresh(&self) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    #[cfg_attr(test, mutants::skip)]
    fn stop_refresh(&self) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }
}

impl DatabaseClient for PostgresTokenClient<'_> {
    type Attribute = Pool;

    fn attribute(&self, name: &str) -> Result<Option<Attribute<Pool>>, Infallible> {
        if name == TOKEN_TABLE_ATTRIBUTE {
            return Ok(self.store.pool.clone().map(Attribute::Model));
        }
        Ok(None)
    }

    #[cfg_attr(test, mutants::skip)]
    fn token_refresh_enabled(&self) -> bool {
        false
    }

    #[cfg_attr(test, mutants::skip)]
    fn token_label(&self) -> &'static str {
        "database token"
    }

    #[cfg_attr(test, mutants::skip)]
    fn minted_url(&self) -> Result<Option<SecretString>, Infallible> {
        Ok(None)
    }
}
