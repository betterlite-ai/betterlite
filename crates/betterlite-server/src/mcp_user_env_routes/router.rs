use axum::Extension;
use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, Method};
use axum::routing::{MethodFilter, get, on};
use betterlite_mcp::registry::resolution::ServerAccessCheck;

use crate::auth::auth_identity::AuthContext;
use crate::mcp::management_server_access::AccessibleServerSources;

use super::UserEnvRoutesState;
use super::body::decode_user_env_vars_request;
use super::error::RouteError;
use super::flows::{
    UserEnvSubmission, authorize_target, clear_user_env_values, list_statuses,
    store_user_env_values,
};
use super::status::compute_user_env_status;
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
    let server_methods = MethodFilter::GET
        .or(MethodFilter::HEAD)
        .or(MethodFilter::POST)
        .or(MethodFilter::DELETE);
    Router::new()
        .route(
            SERVER_ENV_VARS_PATH,
            on(server_methods, server_user_env_vars::<Access, Sources>),
        )
        .route(
            USER_ENV_VARS_STATUS_PATH,
            get(list_mcp_user_env_var_status::<Access, Sources>),
        )
        .with_state(state)
}

// FastAPI validates the body before the handler runs, so a 422 never reaches B865's wrapper
async fn server_user_env_vars<Access, Sources>(
    State(state): State<UserEnvRoutesState<Access, Sources>>,
    Path(server_id): Path<String>,
    (method, headers): (Method, HeaderMap),
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<MCPUserEnvVarsStatus>, RouteError<Access, Sources>>
where
    Access: ServerAccessCheck + 'static,
    Sources: AccessibleServerSources + 'static,
{
    // on() routes only GET, HEAD, POST and DELETE here, so the final branch is the read
    let submission = if method == Method::POST {
        let request = decode_user_env_vars_request(headers.get(CONTENT_TYPE), &body)?;
        UserEnvSubmission::Store(request)
    } else if method == Method::DELETE {
        UserEnvSubmission::Clear
    } else {
        UserEnvSubmission::Read
    };
    let function_name = match &submission {
        UserEnvSubmission::Read => GET_USER_ENV_VARS,
        UserEnvSubmission::Store(_) => STORE_USER_ENV_VARS,
        UserEnvSubmission::Clear => CLEAR_USER_ENV_VARS,
    };
    let routes = &state;
    let status = state
        .manage(caller, function_name, move |auth| async move {
            let (user_id, server) = authorize_target(routes, &auth, &server_id).await?;
            let stored = match submission {
                UserEnvSubmission::Read => {
                    routes.store.get_user_env_vars(&user_id, &server_id).await?
                }
                UserEnvSubmission::Clear => {
                    clear_user_env_values(routes, &user_id, &server_id).await?
                }
                UserEnvSubmission::Store(request) => {
                    store_user_env_values(routes, &user_id, &server_id, &server, &request).await?
                }
            };
            let computed =
                compute_user_env_status(&server, &stored, routes.proxy_base_url.as_deref())?;
            Ok(computed)
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
    state
        .manage(caller, LIST_USER_ENV_VAR_STATUS, move |auth| async move {
            list_statuses(routes, &auth).await
        })
        .await
        .map(Json)
}
