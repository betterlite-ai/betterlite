use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::response::{IntoResponse, Response};
use betterlite_core::api::claude_plugin_write::ClaudePluginWriteResult;

use super::ClaudePluginWriteState;
use super::error::ClaudePluginWriteError;
use super::flow;
use super::lifecycle;
use super::wire::PluginPath;
use crate::auth::auth_identity::AuthContext;

pub(crate) async fn create_plugin(
    State(state): State<Arc<ClaudePluginWriteState>>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Response {
    respond(flow::create(state.as_ref(), caller, &body).await)
}

pub(crate) async fn update_plugin(
    State(state): State<Arc<ClaudePluginWriteState>>,
    caller: Option<Extension<AuthContext>>,
    Path(path): Path<PluginPath>,
    body: Bytes,
) -> Response {
    respond(flow::update(state.as_ref(), caller, path.plugin_name, &body).await)
}

pub(crate) async fn enable_plugin(
    State(state): State<Arc<ClaudePluginWriteState>>,
    caller: Option<Extension<AuthContext>>,
    Path(path): Path<PluginPath>,
) -> Response {
    respond(lifecycle::set_enabled(state.as_ref(), caller, path.plugin_name, true).await)
}

pub(crate) async fn disable_plugin(
    State(state): State<Arc<ClaudePluginWriteState>>,
    caller: Option<Extension<AuthContext>>,
    Path(path): Path<PluginPath>,
) -> Response {
    respond(lifecycle::set_enabled(state.as_ref(), caller, path.plugin_name, false).await)
}

pub(crate) async fn delete_plugin(
    State(state): State<Arc<ClaudePluginWriteState>>,
    caller: Option<Extension<AuthContext>>,
    Path(path): Path<PluginPath>,
) -> Response {
    respond(lifecycle::remove(state.as_ref(), caller, path.plugin_name).await)
}

fn respond(result: Result<ClaudePluginWriteResult, ClaudePluginWriteError>) -> Response {
    match result {
        Ok(body) => Json(body).into_response(),
        Err(error) => error.into_response(),
    }
}
