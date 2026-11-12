mod access_group_read;
mod access_group_write;
mod agent_daily_activity;
mod agent_discovery;
mod anthropic_messages;
mod assistant_object;
mod assistant_runs;
mod assistant_thread;
mod audio;
mod auto_router_preview;
mod auto_router_read_api;
mod batch;
mod batch_create;
mod batch_list;
mod batch_read;
mod budget_api;
mod bulk_user;
mod callback_replay;
pub(crate) mod chat;
mod claude_gateway_oauth;
mod claude_gateway_settings;
mod claude_plugin_access;
mod claude_plugin_read;
mod claude_plugin_write;
mod claude_skill_read;
mod claude_skill_write;
mod completions;
mod container_files;
mod containers;
mod credentials;
mod cursor_responses_compat;
mod customer_create;
mod customer_read;
mod customer_update;
mod debug_report;
pub(crate) mod embeddings;
mod file_content;
mod file_read;
mod file_upload;
mod image;
mod jwt_mapping_api;
mod key_block;
mod key_bulk_update;
mod key_delete;
mod key_info;
mod key_list;
mod key_management;
mod key_regenerate;
mod key_update;
mod login;
mod managed_file;
mod management_v1;
mod master_key_rotation;
mod mcp_byok_consent;
mod mcp_byok_oauth;
mod messages_token;
mod model_block;
mod model_create;
mod model_group_budget;
mod model_group_read;
mod model_group_write;
mod model_info_v1;
mod model_info_v2;
mod model_management;
mod model_patch;
mod model_prompt_preview;
mod model_public_settings;
mod model_write;
mod moderations;
mod ocr;
mod organization_activity;
mod organization_create;
mod organization_members;
mod organization_patch_v2;
mod organization_read;
mod organization_update;
mod prompt_tools;
mod prompts;
mod realtime;
mod realtime_secret;
mod realtime_websocket;
mod rerank;
mod responses;
mod responses_input_tokens;
mod responses_read;
mod responses_ws_auth;
mod router_preview_validation;
mod router_settings;
mod shadow_eval_lifecycle;
mod shadow_eval_results;
mod tag_reads;
mod tag_writes;
mod team_activity;
mod team_callbacks;
mod team_create;
mod team_delete;
mod team_info;
mod team_lists;
mod team_logging_controls;
mod team_member_bulk;
mod team_member_delete;
mod team_member_resets;
mod team_member_writes;
mod team_merge_patch;
mod team_models;
mod team_state;
mod team_update;
pub(crate) mod tests;
mod tool_policy;
mod tool_reports;
mod ui_sessions;
mod user_activity;
mod user_bulk_update;
mod user_delete;
mod user_fields;
mod user_info;
mod user_list;
mod user_management;
mod user_search_scope;
mod user_update;
mod video_character;
mod video_content_remix;
mod video_create;
mod video_edit;

use crate::auth::auth_route_patterns::{self, AuthSettings};
use crate::auth::route_match::stage;
use crate::error_body::json_error;
use crate::model_list::model_routes;
use crate::state::ServerState;
use axum::Json;
use axum::Router;
use axum::error_handling::HandleErrorLayer;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tower::ServiceBuilder;
use tower::timeout::TimeoutLayer;

pub(crate) fn router(
    state: Arc<ServerState>,
    shutting_down: Arc<AtomicBool>,
    chat: Router,
    embeddings: Router,
) -> Router {
    let auth = AuthSettings {
        master_key: state.config.general_settings.master_key.clone(),
        routes: state.config.sections.auth_route_patterns.clone(),
    };
    let application = base_application(shutting_down)
        .merge(model_routes(&state.config.model_list))
        .merge(chat)
        .merge(embeddings)
        .merge(stub_routes(&state));
    application
        .layer(middleware::from_fn_with_state(
            auth,
            auth_route_patterns::stage,
        ))
        .layer(middleware::from_fn(stage))
        .layer(Extension(state))
}

fn base_application(shutting_down: Arc<AtomicBool>) -> Router {
    let timeout_layers = ServiceBuilder::new()
        .layer(HandleErrorLayer::new(|_: tower::BoxError| async {
            json_error(
                StatusCode::REQUEST_TIMEOUT,
                "request timed out",
                "invalid_request_error",
                "408",
            )
        }))
        .layer(TimeoutLayer::new(Duration::from_secs(30)));
    Router::new()
        .route("/health/liveliness", get(health_liveliness))
        .route("/health/liveness", get(health_liveliness))
        .route("/health/readiness", get(health_readiness))
        .with_state(shutting_down)
        .fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                Json(NotFoundBody {
                    detail: "Not Found",
                }),
            )
        })
        .layer(timeout_layers)
}

async fn health_liveliness(State(shutting_down): State<Arc<AtomicBool>>) -> Response {
    if shutting_down.load(Ordering::Acquire) {
        return shutdown_health_response();
    }
    Json("I'm alive!").into_response()
}
async fn health_readiness(State(shutting_down): State<Arc<AtomicBool>>) -> Response {
    if shutting_down.load(Ordering::Acquire) {
        return shutdown_health_response();
    }
    Json(ReadinessBody { status: "healthy" }).into_response()
}
fn shutdown_health_response() -> Response {
    let body = ShutdownBody {
        status: "shutting_down",
    };
    (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
}
#[derive(Serialize)]
struct ShutdownBody {
    status: &'static str,
}
#[derive(Serialize)]
struct ReadinessBody {
    status: &'static str,
}
#[derive(Serialize)]
struct NotFoundBody {
    detail: &'static str,
}

// the stub builders exist so every route mount stays in one short function
fn stub_routes(state: &Arc<ServerState>) -> Router {
    Router::new()
        .merge(stubs_0(state))
        .merge(stubs_1(state))
        .merge(stubs_2(state))
        .merge(stubs_3(state))
}

fn stubs_0(state: &Arc<ServerState>) -> Router {
    Router::new()
        .merge(access_group_read::routes(state))
        .merge(assistant_runs::routes(state))
        .merge(auto_router_read_api::routes(state))
        .merge(batch_read::routes(state))
        .merge(claude_gateway_oauth::routes(state))
        .merge(claude_plugin_write::routes(state))
        .merge(credentials::routes(state))
        .merge(customer_update::routes(state))
        .merge(file_upload::routes(state))
        .merge(key_bulk_update::routes(state))
        .merge(key_management::routes(state))
        .merge(managed_file::routes(state))
        .merge(messages_token::routes(state))
        .merge(model_group_read::routes(state))
        .merge(model_management::routes(state))
        .merge(model_write::routes(state))
        .merge(organization_create::routes(state))
        .merge(organization_update::routes(state))
        .merge(rerank::routes(state))
        .merge(responses_ws_auth::routes(state))
        .merge(shadow_eval_results::routes(state))
        .merge(team_callbacks::routes(state))
        .merge(team_lists::routes(state))
        .merge(team_member_resets::routes(state))
        .merge(team_state::routes(state))
        .merge(ui_sessions::routes(state))
        .merge(user_fields::routes(state))
        .merge(user_search_scope::routes(state))
        .merge(video_create::routes(state))
}

fn stubs_1(state: &Arc<ServerState>) -> Router {
    Router::new()
        .merge(access_group_write::routes(state))
        .merge(assistant_thread::routes(state))
        .merge(batch::routes(state))
        .merge(budget_api::routes(state))
        .merge(claude_gateway_settings::routes(state))
        .merge(claude_skill_read::routes(state))
        .merge(cursor_responses_compat::routes(state))
        .merge(debug_report::routes(state))
        .merge(image::routes(state))
        .merge(key_delete::routes(state))
        .merge(key_regenerate::routes(state))
        .merge(management_v1::routes(state))
        .merge(model_block::routes(state))
        .merge(model_group_write::routes(state))
        .merge(model_patch::routes(state))
        .merge(moderations::routes(state))
        .merge(organization_members::routes(state))
        .merge(realtime::routes(state))
        .merge(responses::routes(state))
        .merge(router_preview_validation::routes(state))
        .merge(tag_reads::routes(state))
        .merge(team_create::routes(state))
        .merge(team_logging_controls::routes(state))
        .merge(team_member_writes::routes(state))
        .merge(team_update::routes(state))
        .merge(user_activity::routes(state))
        .merge(user_info::routes(state))
        .merge(user_update::routes(state))
        .merge(video_edit::routes(state))
}

fn stubs_2(state: &Arc<ServerState>) -> Router {
    Router::new()
        .merge(anthropic_messages::routes(state))
        .merge(audio::routes(state))
        .merge(batch_create::routes(state))
        .merge(bulk_user::routes(state))
        .merge(claude_plugin_access::routes(state))
        .merge(claude_skill_write::routes(state))
        .merge(customer_create::routes(state))
        .merge(file_content::routes(state))
        .merge(jwt_mapping_api::routes(state))
        .merge(key_info::routes(state))
        .merge(key_update::routes(state))
        .merge(master_key_rotation::routes(state))
        .merge(model_create::routes(state))
        .merge(model_info_v1::routes(state))
        .merge(model_prompt_preview::routes(state))
        .merge(ocr::routes(state))
        .merge(organization_patch_v2::routes(state))
        .merge(realtime_secret::routes(state))
        .merge(responses_input_tokens::routes(state))
        .merge(router_settings::routes(state))
        .merge(tag_writes::routes(state))
        .merge(team_delete::routes(state))
        .merge(team_member_bulk::routes(state))
        .merge(team_merge_patch::routes(state))
        .merge(tool_policy::routes(state))
        .merge(user_bulk_update::routes(state))
        .merge(user_list::routes(state))
        .merge(video_character::routes(state))
}

fn stubs_3(state: &Arc<ServerState>) -> Router {
    Router::new()
        .merge(assistant_object::routes(state))
        .merge(auto_router_preview::routes(state))
        .merge(batch_list::routes(state))
        .merge(callback_replay::routes(state))
        .merge(claude_plugin_read::routes(state))
        .merge(completions::routes(state))
        .merge(customer_read::routes(state))
        .merge(file_read::routes(state))
        .merge(key_block::routes(state))
        .merge(key_list::routes(state))
        .merge(login::routes(state))
        .merge(mcp_byok_consent::routes(state))
        .merge(model_group_budget::routes(state))
        .merge(model_info_v2::routes(state))
        .merge(model_public_settings::routes(state))
        .merge(organization_activity::routes(state))
        .merge(organization_read::routes(state))
        .merge(realtime_websocket::routes(state))
        .merge(responses_read::routes(state))
        .merge(shadow_eval_lifecycle::routes(state))
        .merge(team_activity::routes(state))
        .merge(team_info::routes(state))
        .merge(team_member_delete::routes(state))
        .merge(team_models::routes(state))
        .merge(tool_reports::routes(state))
        .merge(user_delete::routes(state))
        .merge(user_management::routes(state))
        .merge(video_content_remix::routes(state))
}
