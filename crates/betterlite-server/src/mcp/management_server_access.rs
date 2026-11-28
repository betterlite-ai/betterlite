use std::collections::{BTreeMap, BTreeSet};
use std::error::Error as StdError;
use std::future::Future;

use betterlite_auth::resolved_key::auth::{UserAPIKeyAuth, user_api_key_has_admin_view};
use betterlite_auth::virtual_key_routes::caller::{AllowedRoutes, RouteCaller};
use betterlite_config::sections::mcp_management_mode::ManagementMode;
use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_mcp::registry::resolution::{
    AuthorizeDetails, AuthorizeError, AuthorizeOptions, DbLookup, NonAdminMissing, ResolveError,
    ResolveTarget, ResolvedServer, ServerAccessCheck, authorize_server, resolve_server,
};
use betterlite_mcp::registry::servers::ServerRegistry;
use betterlite_mcp::registry::table::ServerTables as _;
use thiserror::Error;

mod tests;

#[derive(Debug, Error)]
pub enum ManagementAccessError<Failure>
where
    Failure: StdError + Send + Sync + 'static,
{
    #[error("the MCP server was not found")]
    NotFound(BTreeMap<String, String>),
    #[error("the caller may not manage this MCP server")]
    Forbidden(BTreeMap<String, String>),
    #[error("the MCP server could not be resolved")]
    Lookup(#[source] ResolveError),
    #[error("the MCP server access check failed")]
    Access(#[source] Failure),
}

// the two reads _resolve_accessible_mcp_servers makes beyond the registry, owned by B10127 and B6591
pub trait AccessibleServerSources: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn effective_auth_contexts(
        &self,
        auth: &UserAPIKeyAuth,
    ) -> impl Future<Output = Result<Vec<UserAPIKeyAuth>, Self::Error>> + Send;

    fn allowed_server_tables(
        &self,
        auth: &UserAPIKeyAuth,
    ) -> impl Future<Output = Result<Vec<McpServerTable>, Self::Error>> + Send;
}

impl<Failure> From<AuthorizeError<Failure>> for ManagementAccessError<Failure>
where
    Failure: StdError + Send + Sync + 'static,
{
    fn from(error: AuthorizeError<Failure>) -> Self {
        match error {
            AuthorizeError::NotFound(detail) => Self::NotFound(detail),
            AuthorizeError::Forbidden(detail) => Self::Forbidden(detail),
            AuthorizeError::Access(source) => Self::Access(source),
        }
    }
}

#[must_use]
// ast-grep-ignore: no-long-names - LiteLLM's _is_restricted_virtual_key_request, the name the task sets
pub fn is_restricted_virtual_key_request(auth: &UserAPIKeyAuth) -> bool {
    match RouteCaller::of_key(auth).allowed_routes {
        AllowedRoutes::Listed(routes) => !routes.is_empty(),
        AllowedRoutes::Unset | AllowedRoutes::NotAList => false,
    }
}

// LiteLLM's _authorize_and_fetch_mcp_server: the database row, then the registry, then the access check
pub async fn authorize_and_fetch_mcp_server<Access>(
    registry: &ServerRegistry,
    db: &dyn DbLookup,
    access: &Access,
    auth: &UserAPIKeyAuth,
    server_id: &str,
) -> Result<McpServerTable, ManagementAccessError<Access::Error>>
where
    Access: ServerAccessCheck,
{
    let resolved = resolve_server(
        registry,
        ResolveTarget {
            server_id,
            id_client_ip: None,
            name_client_ip: None,
            match_name: false,
        },
        None,
        Some(db),
    )
    .await
    .map_err(ManagementAccessError::Lookup)?;
    let authorized = authorize_resolved(resolved, access, auth, server_id).await?;
    Ok(authorized.table)
}

async fn authorize_resolved<Access>(
    resolved: Option<ResolvedServer>,
    access: &Access,
    auth: &UserAPIKeyAuth,
    server_id: &str,
) -> Result<ResolvedServer, ManagementAccessError<Access::Error>>
where
    Access: ServerAccessCheck,
{
    let not_found = BTreeMap::from([(
        "error".to_owned(),
        format!("MCP Server {server_id} not found"),
    )]);
    let forbidden = BTreeMap::from([(
        "error".to_owned(),
        format!(
            "User does not have permission to access mcp server with id {server_id}. You can only manage mcp servers that you have access to."
        ),
    )]);
    authorize_server(
        resolved,
        auth,
        access,
        AuthorizeOptions {
            is_admin_view: user_api_key_has_admin_view(auth),
            allow_catalog_view: false,
            non_admin_missing: NonAdminMissing::Forbidden,
        },
        AuthorizeDetails {
            not_found,
            forbidden,
        },
    )
    .await
    .map_err(ManagementAccessError::from)
}

// LiteLLM's _resolve_accessible_mcp_servers: every registry server in view_all, else the first-seen union
pub async fn resolve_accessible_mcp_servers<Sources>(
    registry: &ServerRegistry,
    sources: &Sources,
    auth: &UserAPIKeyAuth,
    mode: ManagementMode,
) -> Result<Vec<McpServerTable>, Sources::Error>
where
    Sources: AccessibleServerSources,
{
    // ast-grep-ignore: no-long-names - the LiteLLM predicate this task owns
    if mode == ManagementMode::ViewAll && !is_restricted_virtual_key_request(auth) {
        return Ok(registry.all_server_tables());
    }
    let mut seen_ids = BTreeSet::new();
    let mut servers = Vec::new();
    for context in sources.effective_auth_contexts(auth).await? {
        for table in sources.allowed_server_tables(&context).await? {
            if seen_ids.insert(table.server_id.clone()) {
                servers.push(table);
            }
        }
    }
    Ok(servers)
}
