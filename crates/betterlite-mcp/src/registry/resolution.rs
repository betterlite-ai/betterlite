use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::future::Future;
use std::pin::Pin;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp::table_access::types::McpTableError;
use thiserror::Error;

use super::servers::ServerRegistry;
use crate::server_build::BuildError;
use crate::server_record::MCPServer;

mod tests;

pub type LookupFuture<'lookup, Item> =
    Pin<Box<dyn Future<Output = Result<Item, ResolveError>> + Send + 'lookup>>;

#[derive(Debug, Error)]
pub enum ResolveError {
    #[error("the MCP server row could not be read")]
    Table(#[source] Box<McpTableError>),
    #[error("the MCP server record could not be built")]
    Build(#[source] Box<BuildError>),
}

impl From<McpTableError> for ResolveError {
    fn from(error: McpTableError) -> Self {
        Self::Table(Box::new(error))
    }
}

impl From<BuildError> for ResolveError {
    fn from(error: BuildError) -> Self {
        Self::Build(Box::new(error))
    }
}

// LiteLLM's temp_lookup: the throwaway servers a session created, which are never a database row
pub trait TempLookup: Send + Sync {
    fn temp_server<'lookup>(
        &'lookup self,
        server_id: &'lookup str,
    ) -> LookupFuture<'lookup, Option<MCPServer>>;
}

// LiteLLM's db_lookup: the stored row for a server id, read before the in-memory registry
pub trait DbLookup: Send + Sync {
    fn db_table<'lookup>(
        &'lookup self,
        server_id: &'lookup str,
    ) -> LookupFuture<'lookup, Option<McpServerTable>>;
}

// LiteLLM's can_access_mcp_server(user_api_key_dict, server_id, get_allowed_mcp_servers), owned by B4672
pub trait ServerAccessCheck: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn can_access(
        &self,
        auth: &UserAPIKeyAuth,
        server_id: &str,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send;
}

// LiteLLM's ResolutionSource
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolutionSource {
    Temp,
    Db,
    Registry,
}

// LiteLLM's ResolvedMCPServer
#[derive(Clone, Debug)]
pub struct ResolvedServer {
    pub table: McpServerTable,
    pub runtime: Option<MCPServer>,
    pub source: ResolutionSource,
}

// The server id and the two ip gates resolve_mcp_server takes
#[derive(Clone, Copy, Debug)]
pub struct ResolveTarget<'target> {
    pub server_id: &'target str,
    pub id_client_ip: Option<&'target str>,
    pub name_client_ip: Option<&'target str>,
    pub match_name: bool,
}

// LiteLLM's non_admin_missing: which denial an unresolved server gives a caller who is not an admin
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NonAdminMissing {
    NotFound,
    Forbidden,
}

// The flags authorize_mcp_server takes besides the caller and the resolution
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorizeOptions {
    pub is_admin_view: bool,
    pub allow_catalog_view: bool,
    pub non_admin_missing: NonAdminMissing,
}

// The two JSON bodies the route layer renders for a 404 and a 403
#[derive(Clone, Debug)]
pub struct AuthorizeDetails {
    pub not_found: BTreeMap<String, String>,
    pub forbidden: BTreeMap<String, String>,
}

// LiteLLM raises HTTPException; the route layer renders these as the same 404 and 403 bodies
#[derive(Debug, Error)]
pub enum AuthorizeError<Failure>
where
    Failure: StdError + Send + Sync + 'static,
{
    #[error("MCP server not found")]
    NotFound(BTreeMap<String, String>),
    #[error("MCP server access denied")]
    Forbidden(BTreeMap<String, String>),
    #[error("the MCP server access check failed")]
    Access(#[source] Failure),
}

// LiteLLM's resolve_mcp_server: temp, then the database row, then the registry, then the name match
pub async fn resolve_server(
    registry: &ServerRegistry,
    target: ResolveTarget<'_>,
    temp_lookup: Option<&dyn TempLookup>,
    db_lookup: Option<&dyn DbLookup>,
) -> Result<Option<ResolvedServer>, ResolveError> {
    if let Some(lookup) = temp_lookup
        && let Some(runtime) = lookup.temp_server(target.server_id).await?
    {
        return Ok(Some(with_runtime(
            registry,
            runtime,
            ResolutionSource::Temp,
        )));
    }
    if let Some(lookup) = db_lookup
        && let Some(table) = lookup.db_table(target.server_id).await?
    {
        return Ok(Some(ResolvedServer {
            table,
            runtime: None,
            source: ResolutionSource::Db,
        }));
    }
    Ok(resolve_registry(registry, target))
}

fn resolve_registry(
    registry: &ServerRegistry,
    target: ResolveTarget<'_>,
) -> Option<ResolvedServer> {
    let found = registry
        .server_by_id(target.server_id, None)
        .filter(|server| {
            target.id_client_ip.is_none()
                || registry.is_accessible_from_ip(server, target.id_client_ip)
        });
    if let Some(server) = found {
        return Some(with_runtime(
            registry,
            server.clone(),
            ResolutionSource::Registry,
        ));
    }
    if target.match_name
        && let Some(server) = registry.server_by_name(target.server_id, target.name_client_ip)
    {
        return Some(with_runtime(
            registry,
            server.clone(),
            ResolutionSource::Registry,
        ));
    }
    None
}

// LiteLLM's authorize_mcp_server: a missing resolution, the admin bypass and the access check
pub async fn authorize_server<Checker>(
    resolved: Option<ResolvedServer>,
    auth: &UserAPIKeyAuth,
    access: &Checker,
    options: AuthorizeOptions,
    details: AuthorizeDetails,
) -> Result<ResolvedServer, AuthorizeError<Checker::Error>>
where
    Checker: ServerAccessCheck,
{
    let Some(resolution) = resolved else {
        if options.is_admin_view || options.non_admin_missing == NonAdminMissing::NotFound {
            return Err(AuthorizeError::NotFound(details.not_found));
        }
        return Err(AuthorizeError::Forbidden(details.forbidden));
    };
    if options.is_admin_view {
        return Ok(resolution);
    }
    if resolution.source == ResolutionSource::Temp {
        return Err(AuthorizeError::Forbidden(details.forbidden));
    }
    if !options.allow_catalog_view {
        let allowed = access
            .can_access(auth, &resolution.table.server_id)
            .await
            .map_err(AuthorizeError::Access)?;
        if !allowed {
            return Err(AuthorizeError::Forbidden(details.forbidden));
        }
    }
    Ok(resolution)
}

fn with_runtime(
    registry: &ServerRegistry,
    server: MCPServer,
    source: ResolutionSource,
) -> ResolvedServer {
    let table = registry.server_table(&server);
    ResolvedServer {
        table,
        runtime: Some(server),
        source,
    }
}
