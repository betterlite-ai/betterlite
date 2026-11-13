use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse as _, Response};
use betterlite_routes::{GATEWAY_EXACT_PATHS, GATEWAY_MOUNT_PATHS, GATEWAY_PATH_PREFIXES};
use serde::Serialize;

#[derive(Serialize)]
struct NotFoundBody {
    detail: &'static str,
}

// LiteLLM keeps a route when it is a kept mount, an exact path, or under a kept prefix
pub fn allows(path: &str) -> bool {
    GATEWAY_MOUNT_PATHS.contains(path)
        || GATEWAY_EXACT_PATHS.contains(path)
        || GATEWAY_PATH_PREFIXES
            .iter()
            .any(|prefix| path.starts_with(prefix))
}

pub async fn stage(request: Request, next: Next) -> Response {
    if allows(request.uri().path()) {
        next.run(request).await
    } else {
        (
            StatusCode::NOT_FOUND,
            axum::Json(NotFoundBody {
                detail: "Not Found",
            }),
        )
            .into_response()
    }
}
