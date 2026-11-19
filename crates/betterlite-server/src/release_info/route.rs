use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;

use super::records::{LatestRelease, LatestReleaseInfo, LatestReleaseUnavailable};
use super::service::{ReleaseError, ReleaseService};
use crate::error_body::json_error;
use crate::roi_github::transport::Transport;

pub(super) const LATEST_RELEASE_PATH: &str = "/get/latest_release_info";

pub(super) fn router<Client>(service: Arc<ReleaseService<Client>>) -> Router
where
    Client: Transport + 'static,
{
    Router::new()
        .route(LATEST_RELEASE_PATH, get(latest_release_info::<Client>))
        .with_state(service)
}

async fn latest_release_info<Client>(State(service): State<Arc<ReleaseService<Client>>>) -> Response
where
    Client: Transport + 'static,
{
    match service.get().await {
        Ok(LatestRelease::Available(info)) => Json(*info).into_response(),
        Ok(LatestRelease::Unavailable(unavailable)) => unavailable_response(&unavailable),
        Err(error) => failed_response(&error),
    }
}

fn unavailable_response(unavailable: &LatestReleaseUnavailable) -> Response {
    tracing::warn!(reason = %unavailable.reason, "latest release info unavailable");
    Json(Option::<LatestReleaseInfo>::None).into_response()
}

fn failed_response(error: &ReleaseError) -> Response {
    tracing::error!(error = %error, "the latest release info could not be read");
    json_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error",
        "internal_server_error",
        "500",
    )
}
