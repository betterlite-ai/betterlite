use axum::Json;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::auth::auth_identity::AuthContext;
use crate::reload_routes::ReloadRoutesState;

use super::beta_config::{provider_count, write_force_reload_row};
use super::caller::{caller_principal, require_proxy_admin};
use super::error::{ReloadError, WrappedCause};
use super::wire::naive_utc_isoformat;

const RELOAD_FAILED: &str = "Failed to reload anthropic beta headers";

#[derive(Serialize)]
struct ReloadBody {
    message: String,
    status: &'static str,
    providers_count: usize,
    timestamp: String,
}

pub(super) async fn reload_anthropic_beta_headers(
    State(state): State<ReloadRoutesState>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Response, ReloadError> {
    let principal = caller_principal(caller)?;
    require_proxy_admin(&principal)?;
    let Some(store) = state.config.as_deref() else {
        return Err(ReloadError::Wrapped {
            prefix: RELOAD_FAILED,
            cause: WrappedCause::MissingDatabase,
        });
    };
    let matrix = state.beta_headers.reload().await;
    let now = (state.clock)();
    state.reloads.set_last_beta_headers_reload(now);
    write_force_reload_row(store)
        .await
        .map_err(|source| ReloadError::Wrapped {
            prefix: RELOAD_FAILED,
            cause: WrappedCause::Store(source),
        })?;
    let providers_count = provider_count(&matrix);
    tracing::info!(
        "Anthropic beta headers config reloaded successfully in current pod. Providers: {providers_count}"
    );
    let body = ReloadBody {
        message: format!(
            "Anthropic beta headers configuration reloaded successfully! {providers_count} providers updated."
        ),
        status: "success",
        providers_count,
        timestamp: naive_utc_isoformat(now),
    };
    Ok(Json(body).into_response())
}
