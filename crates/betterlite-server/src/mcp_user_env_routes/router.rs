use axum::Extension;
use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::http::header::CONTENT_TYPE;
use axum::routing::get;
use betterlite_mcp::registry::resolution::ServerAccessCheck;

use crate::auth::auth_identity::AuthContext;
use crate::mcp::management_server_access::AccessibleServerSources;

use super::UserEnvRoutesState;
use super::body::decode_user_env_vars_request;
use super::error::RouteError;
use super::flows::{clear_values, list_statuses, read_status, store_values};
use super::types::MCPUserEnvVarsStatus;

pub(crate) const SERVER_ENV_VARS_PATH: &str = "/v1/mcp/server/{server_id}/user-env-vars";
pub(crate) const USER_ENV_VARS_STATUS_PATH: &str = "/v1/mcp/user-env-vars/status";

const GET_USER_ENV_VARS: &str = "get_mcp_user_env_vars";
const STORE_USER_ENV_VARS: &str = "store_mcp_user_env_vars";
const CLEAR_USER_ENV_VARS: &str = "clear_mcp_user_env_vars";
const LIST_USER_ENV_VAR_STATUS: &str = "list_mcp_user_env_var_status";

// LiteLLM mounts the management router at /v1/mcp only, so these paths carry the whole prefix
pub(crate) fn routes<Access, Sources>(state: UserEnvRoutesState<Access, Sources>) -> Router
where
    Access: ServerAccessCheck + 'static,
    Sources: AccessibleServerSources + 'static,
{
    Router::new()
        .route(
            SERVER_ENV_VARS_PATH,
            get(get_mcp_user_env_vars::<Access, Sources>)
                .post(store_mcp_user_env_vars::<Access, Sources>)
                .delete(clear_mcp_user_env_vars::<Access, Sources>),
        )
        .route(
            USER_ENV_VARS_STATUS_PATH,
            get(list_mcp_user_env_var_status::<Access, Sources>),
        )
        .with_state(state)
}

async fn get_mcp_user_env_vars<Access, Sources>(
    State(state): State<UserEnvRoutesState<Access, Sources>>,
    Path(server_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<MCPUserEnvVarsStatus>, RouteError<Access, Sources>>
where
    Access: ServerAccessCheck + 'static,
    Sources: AccessibleServerSources + 'static,
{
    let routes = &state;
    let status = state
        .manage(caller, GET_USER_ENV_VARS, move |auth| async move {
            read_status(routes, &auth, &server_id).await
        })
        .await?;
    Ok(Json(status))
}

// FastAPI validates the body before the handler runs, so a 422 never reaches B865's wrapper
async fn store_mcp_user_env_vars<Access, Sources>(
    State(state): State<UserEnvRoutesState<Access, Sources>>,
    Path(server_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<MCPUserEnvVarsStatus>, RouteError<Access, Sources>>
where
    Access: ServerAccessCheck + 'static,
    Sources: AccessibleServerSources + 'static,
{
    let request = decode_user_env_vars_request(headers.get(CONTENT_TYPE), &body)?;
    let routes = &state;
    let status = state
        .manage(caller, STORE_USER_ENV_VARS, move |auth| async move {
            store_values(routes, &auth, &server_id, &request).await
        })
        .await?;
    Ok(Json(status))
}

async fn clear_mcp_user_env_vars<Access, Sources>(
    State(state): State<UserEnvRoutesState<Access, Sources>>,
    Path(server_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<MCPUserEnvVarsStatus>, RouteError<Access, Sources>>
where
    Access: ServerAccessCheck + 'static,
    Sources: AccessibleServerSources + 'static,
{
    let routes = &state;
    let status = state
        .manage(caller, CLEAR_USER_ENV_VARS, move |auth| async move {
            clear_values(routes, &auth, &server_id).await
        })
        .await?;
    Ok(Json(status))
}

async fn list_mcp_user_env_var_status<Access, Sources>(
    State(state): State<UserEnvRoutesState<Access, Sources>>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<Vec<MCPUserEnvVarsStatus>>, RouteError<Access, Sources>>
where
    Access: ServerAccessCheck + 'static,
    Sources: AccessibleServerSources + 'static,
{
    let routes = &state;
    let statuses = state
        .manage(caller, LIST_USER_ENV_VAR_STATUS, move |auth| async move {
            list_statuses(routes, &auth).await
        })
        .await?;
    Ok(Json(statuses))
}
