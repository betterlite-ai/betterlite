use axum::Json;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use betterlite_auth::auxiliary_routes;
use betterlite_auth::management_routes;
use betterlite_auth::route_match::classifier;
use betterlite_auth::route_match::templates::{
    RouteValue, check_route_access, route_matches_pattern,
};
use percent_encoding::percent_decode_str;
use serde::Serialize;

mod tests;

const MAX_ROUTE_PATH_BYTES: usize = 8_192;

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
        || check_route_access(route, &management_routes::management_routes())
        || auxiliary_routes::INFO_ROUTES.contains(&route)
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
    let Some(route) = decoded_route_path(request.uri().path()) else {
        return RouteMatchError::Unrecognized.into_response();
    };
    if matches!(
        route.as_str(),
        "/health/liveliness" | "/health/liveness" | "/health/readiness"
    ) {
        return next.run(request).await;
    }
    let method = request.method().as_str().to_owned();
    match admit(&route, &method) {
        Ok(()) => next.run(mark_admitted(request)).await,
        Err(error) => error.into_response(),
    }
}

#[derive(Clone, Copy)]
struct RouteMatchAdmitted;

fn mark_admitted(mut request: Request) -> Request {
    request.extensions_mut().insert(RouteMatchAdmitted);
    request
}

fn decoded_route_path(path: &str) -> Option<String> {
    if path.len() > MAX_ROUTE_PATH_BYTES {
        return None;
    }
    percent_decode_str(path)
        .decode_utf8()
        .ok()
        .map(std::borrow::Cow::into_owned)
}
