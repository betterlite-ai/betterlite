use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::resolved_key::auth::{UserAPIKeyAuth, user_api_key_has_admin_view};
use betterlite_cache::backend::CacheBackend;
use betterlite_db::mcp::table_access::types::McpTableError;
use betterlite_db::mcp_server_encoding::request::McpServerRequest;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_mcp::registry::lookup::ServerLookups as _;
use betterlite_mcp::registry::resolution::{
    AuthorizeDetails, AuthorizeOptions, LookupFuture, NonAdminMissing, ResolveError, ResolveTarget,
    ResolvedServer, ServerAccessCheck, TempLookup, authorize_server, resolve_server,
};
use betterlite_mcp::registry::servers::ServerRegistry;
use betterlite_mcp::server_payload::RequestError;
use betterlite_mcp::server_payload::request::NewMCPServerRequest;
use betterlite_mcp::server_record::MCPServer;
use chrono::{DateTime, Duration, Utc};

use self::ports::{ServerFromTable, SessionDatabase, SessionIds};
use super::SessionError;
// ast-grep-ignore: no-long-names
use super::TEMPORARY_MCP_SERVER_TTL_SECONDS as SESSION_TTL;
use super::redis_copy::read_redis_copy;
use crate::failure_hook::ports::Clock;
use crate::mcp::server_writes::{DRAFT, DraftOptions, ServerWriteError};

pub mod ports;
mod tests;

// LiteLLM's _TemporaryMCPServerEntry
struct LocalEntry {
    server: MCPServer,
    expires_at: DateTime<Utc>,
}

pub struct SessionStoreParts<Access>
where
    Access: ServerAccessCheck,
{
    pub clock: Arc<dyn Clock>,
    pub cipher: Arc<DbCipher>,
    pub redis: Option<Arc<dyn CacheBackend>>,
    pub database: Option<Arc<dyn SessionDatabase>>,
    pub builder: Arc<dyn ServerFromTable>,
    pub ids: Arc<dyn SessionIds>,
    pub access: Access,
}

// LiteLLM's module-level temporary server table, with every collaborator the flow needs as a field
pub struct SessionStore<Access>
where
    Access: ServerAccessCheck,
{
    local: Mutex<HashMap<String, LocalEntry>>,
    pub(super) clock: Arc<dyn Clock>,
    pub(super) cipher: Arc<DbCipher>,
    pub(super) redis: Option<Arc<dyn CacheBackend>>,
    pub(super) database: Option<Arc<dyn SessionDatabase>>,
    pub(super) builder: Arc<dyn ServerFromTable>,
    pub(super) ids: Arc<dyn SessionIds>,
    pub(super) access: Access,
}

impl<Access> SessionStore<Access>
where
    Access: ServerAccessCheck,
{
    #[must_use]
    pub fn new(parts: SessionStoreParts<Access>) -> Self {
        Self {
            local: Mutex::new(HashMap::new()),
            clock: parts.clock,
            cipher: parts.cipher,
            redis: parts.redis,
            database: parts.database,
            builder: parts.builder,
            ids: parts.ids,
            access: parts.access,
        }
    }

    // LiteLLM's _prune_expired_temporary_mcp_servers: expires_at at or before now leaves the table
    fn prune_expired(&self) {
        let now = self.clock.now().with_timezone(&Utc);
        let mut local = self.local.lock().unwrap_or_else(PoisonError::into_inner);
        local.retain(|_, entry| entry.expires_at > now);
    }

    // LiteLLM's _cache_temporary_mcp_server: ttl at least 1, prune, then replace the id's own entry
    #[must_use]
    pub fn cache_local(&self, server: MCPServer, ttl_seconds: i64) -> MCPServer {
        let ttl = ttl_seconds.max(1);
        self.prune_expired();
        let now = self.clock.now().with_timezone(&Utc);
        // a ttl too large to add to now keeps the entry until the clock's own last instant
        let expires_at = Duration::try_seconds(ttl)
            .and_then(|window| now.checked_add_signed(window))
            .unwrap_or(DateTime::<Utc>::MAX_UTC);
        let mut local = self.local.lock().unwrap_or_else(PoisonError::into_inner);
        local.insert(
            server.fields.server_id.clone(),
            LocalEntry {
                server: server.clone(),
                expires_at,
            },
        );
        server
    }

    // LiteLLM's get_cached_temporary_mcp_server: local table, then the shared draft row, then Redis
    pub async fn get_cached_temporary_server(
        &self,
        server_id: &str,
    ) -> Result<Option<MCPServer>, ResolveError> {
        self.prune_expired();
        let cached = self
            .local
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(server_id)
            .map(|entry| entry.server.clone());
        if let Some(server) = cached {
            return Ok(Some(server));
        }
        // a draft or Redis hit is returned as is, so a shared entry cannot outlive its own TTL
        if let Some(server) = self.draft_as_server(server_id).await? {
            return Ok(Some(server));
        }
        Ok(read_redis_copy(self, server_id).await)
    }

    // LiteLLM's _get_draft_mcp_server_as_mcp_server: the row's credentials are encrypted at rest
    pub async fn draft_as_server(
        &self,
        server_id: &str,
    ) -> Result<Option<MCPServer>, ResolveError> {
        let Some(database) = self.database.as_deref() else {
            return Ok(None);
        };
        let found = database
            .get_draft(server_id)
            .await
            .map_err(|error| ResolveError::Table(Box::new(draft_read_failure(server_id, error))))?;
        let Some(row) = found else {
            return Ok(None);
        };
        let server = self.builder.build_server(&row, true).await?;
        Ok(Some(server))
    }

    // LiteLLM's _persist_draft_mcp_server: without a database the session stays in this process only
    pub async fn persist_draft(
        &self,
        request: &NewMCPServerRequest,
        server_id: &str,
        created_by: &str,
    ) -> Result<(), SessionError<Access::Error>> {
        let Some(database) = self.database.as_deref() else {
            return Ok(());
        };
        let draft = draft_request(request)?;
        let options = DraftOptions {
            touched_by: created_by.to_owned(),
            ttl_seconds: SESSION_TTL,
            server_id: Some(server_id.to_owned()),
        };
        database.create_draft(&draft, &options).await?;
        Ok(())
    }

    // LiteLLM's _resolve_session_server_id: a fresh id unless the supplied one names a saved server
    pub async fn resolve_session_id(
        &self,
        request: &NewMCPServerRequest,
        registry: &ServerRegistry,
    ) -> Result<String, SessionError<Access::Error>> {
        let requested = request
            .server_id
            .as_deref()
            .filter(|server_id| !server_id.is_empty());
        let Some(supplied) = requested else {
            return Ok(self.ids.fresh_id());
        };
        if registry.server_by_id(supplied, None).is_some() {
            return Ok(supplied.to_owned());
        }
        let Some(database) = self.database.as_deref() else {
            return Ok(supplied.to_owned());
        };
        // a draft row belongs to another session, so re-supplying its id must not adopt it
        match database.get_server(supplied).await? {
            Some(row) if row.approval_status.as_deref() != Some(DRAFT) => Ok(supplied.to_owned()),
            Some(_) | None => Ok(self.ids.fresh_id()),
        }
    }

    // LiteLLM's _get_cached_temporary_mcp_server_or_404: resolve the session, then authorize the caller
    pub async fn cached_temporary_server_or_404(
        &self,
        server_id: &str,
        auth: &UserAPIKeyAuth,
        client_ip: Option<&str>,
        registry: &ServerRegistry,
    ) -> Result<MCPServer, SessionError<Access::Error>> {
        let resolved = self.session_lookup(server_id, client_ip, registry).await?;
        let not_found = BTreeMap::from([(
            "error".to_owned(),
            format!("MCP server {server_id} not found"),
        )]);
        let forbidden = BTreeMap::from([(
            "error".to_owned(),
            format!("Access denied to MCP server {server_id}"),
        )]);
        let authorized = authorize_server(
            resolved,
            auth,
            &self.access,
            AuthorizeOptions {
                is_admin_view: user_api_key_has_admin_view(auth),
                allow_catalog_view: false,
                non_admin_missing: NonAdminMissing::NotFound,
            },
            AuthorizeDetails {
                not_found,
                forbidden,
            },
        )
        .await?;
        authorized.runtime.ok_or(SessionError::MissingRuntime {
            server_id: server_id.to_owned(),
        })
    }

    // the id first, the name only when the id misses, and only this process's temporary servers count
    async fn session_lookup(
        &self,
        server_id: &str,
        client_ip: Option<&str>,
        registry: &ServerRegistry,
    ) -> Result<Option<ResolvedServer>, ResolveError> {
        let lookup: &dyn TempLookup = self;
        resolve_server(
            registry,
            ResolveTarget {
                server_id,
                id_client_ip: None,
                name_client_ip: client_ip,
                match_name: true,
            },
            Some(lookup),
            None,
        )
        .await
    }
}

impl<Access> TempLookup for SessionStore<Access>
where
    Access: ServerAccessCheck,
{
    fn temp_server<'lookup>(
        &'lookup self,
        server_id: &'lookup str,
    ) -> LookupFuture<'lookup, Option<MCPServer>> {
        Box::pin(async move { self.get_cached_temporary_server(server_id).await })
    }
}

// B4712's draft write takes the row encoder's request type, which carries the same wire names
fn draft_request(request: &NewMCPServerRequest) -> Result<McpServerRequest, RequestError> {
    let encoded = serde_json::to_string(request)?;
    let decoded = serde_json::from_str(&encoded)?;
    Ok(decoded)
}

// a draft read only finds and decodes one row, so the write-path variants cannot come back from it
fn draft_read_failure(server_id: &str, error: ServerWriteError) -> McpTableError {
    match error {
        ServerWriteError::Table(source) => source,
        ServerWriteError::Json(source) => McpTableError::Json(source),
        ServerWriteError::MissingServerId => McpTableError::MissingServerId,
        ServerWriteError::UnreadableRow { .. }
        | ServerWriteError::Encoding(_)
        | ServerWriteError::UnknownColumn { .. }
        | ServerWriteError::InsertedRowMissing => McpTableError::ServerNotFound {
            server_id: server_id.to_owned(),
        },
    }
}
