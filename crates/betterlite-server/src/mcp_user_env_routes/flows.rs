use std::collections::BTreeSet;

use axum::http::StatusCode;
use betterlite_auth::auth_key_lookup::key_identity;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp_user_env_vars::store::UserEnvVars;
use betterlite_mcp::registry::resolution::ServerAccessCheck;
use secrecy::SecretString;

use crate::mcp::management_access::get_prisma_client_or_throw;
use crate::mcp::management_server_access::{
    AccessibleServerSources, ManagementAccessError, authorize_and_fetch_mcp_server,
    resolve_accessible_mcp_servers,
};

use super::UserEnvRoutesState;
use super::error::{DATABASE_NOT_CONNECTED, HttpRefusal, RouteError, UserEnvRouteError};
use super::status::{compute_user_env_status, parse_server_env_vars};
use super::types::{MCPUserEnvVarsRequest, MCPUserEnvVarsStatus};

// The one server-scoped path answers three methods, so the method picks the work B865's wrapper runs
pub(crate) enum UserEnvSubmission {
    Read,
    Store(MCPUserEnvVarsRequest),
    Clear,
}

pub(super) async fn clear_user_env_values<Access, Sources>(
    state: &UserEnvRoutesState<Access, Sources>,
    user_id: &str,
    server_id: &str,
) -> Result<UserEnvVars, RouteError<Access, Sources>>
where
    Access: ServerAccessCheck,
    Sources: AccessibleServerSources,
{
    state.store.delete_user_env_vars(user_id, server_id).await?;
    state.cache.invalidate(user_id, server_id);
    Ok(UserEnvVars::new())
}

pub(super) async fn store_user_env_values<Access, Sources>(
    state: &UserEnvRoutesState<Access, Sources>,
    user_id: &str,
    server_id: &str,
    server: &McpServerTable,
    request: &MCPUserEnvVarsRequest,
) -> Result<UserEnvVars, RouteError<Access, Sources>>
where
    Access: ServerAccessCheck,
    Sources: AccessibleServerSources,
{
    let (_, user_specs) = parse_server_env_vars(server)?;
    let allowed_names: BTreeSet<String> = user_specs.iter().map(|spec| spec.name.clone()).collect();
    let mut updates = UserEnvVars::new();
    for (name, value) in &request.values {
        if allowed_names.contains(name) && !value.is_empty() {
            updates.insert(name.clone(), SecretString::from(value.clone()));
        }
    }
    let merged = state
        .store
        .merge_user_env_vars(user_id, server_id, &updates, &allowed_names)
        .await?;
    state.cache.invalidate(user_id, server_id);
    Ok(merged)
}

pub(crate) async fn list_statuses<Access, Sources>(
    state: &UserEnvRoutesState<Access, Sources>,
    auth: &UserAPIKeyAuth,
) -> Result<Vec<MCPUserEnvVarsStatus>, RouteError<Access, Sources>>
where
    Access: ServerAccessCheck,
    Sources: AccessibleServerSources,
{
    ensure_database(state)?;
    let user_id = key_identity(auth).user_id.unwrap_or_default();
    if user_id.is_empty() {
        return Ok(Vec::new());
    }
    let accessible = {
        let registry = state.registry.read().await;
        resolve_accessible_mcp_servers(&registry, state.sources.as_ref(), auth, state.mode)
            .await
            .map_err(UserEnvRouteError::Sources)?
    };
    if accessible.is_empty() {
        return Ok(Vec::new());
    }
    let server_ids: Vec<String> = accessible
        .iter()
        .map(|server| server.server_id.clone())
        .collect();
    let stored = state
        .store
        .get_user_env_vars_bulk(&user_id, &server_ids)
        .await?;
    let empty = UserEnvVars::new();
    let mut statuses = Vec::new();
    for server in &accessible {
        let values = stored.get(&server.server_id).unwrap_or(&empty);
        let status = compute_user_env_status(server, values, state.proxy_base_url.as_deref())?;
        if !status.required.is_empty() {
            statuses.push(status);
        }
    }
    Ok(statuses)
}

pub(super) async fn authorize_target<Access, Sources>(
    state: &UserEnvRoutesState<Access, Sources>,
    auth: &UserAPIKeyAuth,
    server_id: &str,
) -> Result<(String, McpServerTable), RouteError<Access, Sources>>
where
    Access: ServerAccessCheck,
    Sources: AccessibleServerSources,
{
    ensure_database(state)?;
    let user_id = caller_user_id(auth).map_err(UserEnvRouteError::Refusal)?;
    let registry = state.registry.read().await;
    let resolved = authorize_and_fetch_mcp_server(
        &registry,
        state.db_lookup.as_ref(),
        state.access.as_ref(),
        auth,
        server_id,
    )
    .await;
    drop(registry);
    match resolved {
        Ok(server) => Ok((user_id, server)),
        Err(ManagementAccessError::NotFound(detail)) => Err(UserEnvRouteError::Refusal(
            HttpRefusal::from_detail(StatusCode::NOT_FOUND, detail),
        )),
        Err(ManagementAccessError::Forbidden(detail)) => Err(UserEnvRouteError::Refusal(
            HttpRefusal::from_detail(StatusCode::FORBIDDEN, detail),
        )),
        Err(ManagementAccessError::Lookup(source)) => Err(UserEnvRouteError::Lookup(source)),
        Err(ManagementAccessError::Access(source)) => Err(UserEnvRouteError::Access(source)),
    }
}

// LiteLLM's get_prisma_client_or_throw: the handle is only checked for presence, the store owns the reads
fn ensure_database<Access, Sources>(
    state: &UserEnvRoutesState<Access, Sources>,
) -> Result<(), RouteError<Access, Sources>>
where
    Access: ServerAccessCheck,
    Sources: AccessibleServerSources,
{
    let _database = get_prisma_client_or_throw(state.database.as_deref(), DATABASE_NOT_CONNECTED)
        .map_err(|error| HttpRefusal::from_database(&error))
        .map_err(UserEnvRouteError::Refusal)?;
    Ok(())
}

fn caller_user_id(auth: &UserAPIKeyAuth) -> Result<String, HttpRefusal> {
    key_identity(auth)
        .user_id
        .filter(|user_id| !user_id.is_empty())
        .ok_or_else(HttpRefusal::missing_user_id)
}
