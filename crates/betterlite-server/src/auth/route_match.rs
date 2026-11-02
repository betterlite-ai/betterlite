use axum::Json;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use betterlite_auth::route_match::classifier;
use betterlite_auth::route_match::templates::{RouteValue, route_matches_pattern};
use serde::Serialize;

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum RouteMatchError {
    #[error("route not recognized")]
    Unrecognized,
}

impl IntoResponse for RouteMatchError {
    fn into_response(self) -> Response {
        (
            StatusCode::NOT_FOUND,
            Json(NotFoundBody {
                detail: "Not Found",
            }),
        )
            .into_response()
    }
}

#[derive(Serialize)]
struct NotFoundBody {
    detail: &'static str,
}

pub fn admit(route: &str, method: &str) -> Result<(), RouteMatchError> {
    let recognized = classifier::is_llm_api_route(&RouteValue::Text(route))
        || classifier::is_management_route(route)
        || classifier::is_info_route(route)
        || classifier::is_mcp_discovery_route(route, Some(method))
        || ["/models/{model_id}", "/v1/models/{model_id}"]
            .iter()
            .any(|pattern| route_matches_pattern(&RouteValue::Text(route), pattern));
    if recognized {
        Ok(())
    } else {
        Err(RouteMatchError::Unrecognized)
    }
}

pub(crate) async fn stage(request: Request, next: Next) -> Response {
    let route = request.uri().path().to_owned();
    if matches!(
        route.as_str(),
        "/health/liveliness" | "/health/liveness" | "/health/readiness"
    ) {
        return next.run(request).await;
    }
    let method = request.method().as_str().to_owned();
    match admit(&route, &method) {
        Ok(()) => next.run(request).await,
        Err(error) => error.into_response(),
    }
}
