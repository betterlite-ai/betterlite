use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse as _, Response};
use phf::Set;
use serde::Serialize;

#[derive(Clone, Copy)]
pub struct ComponentRoutes {
    pub prefixes: &'static [&'static str],
    pub exact: &'static Set<&'static str>,
    pub mounts: &'static Set<&'static str>,
}

#[derive(Serialize)]
struct NotFoundBody {
    detail: &'static str,
}

// LiteLLM keeps a route when it is a kept mount, an exact path, or under a kept prefix
pub fn matches_path(path: &str, routes: ComponentRoutes) -> bool {
    routes.mounts.contains(path)
        || routes.exact.contains(path)
        || routes
            .prefixes
            .iter()
            .any(|prefix| prefix_matches(path, prefix))
}

// a prefix without a template is Python's startswith; a {name} segment keeps Starlette matching one non-empty segment
fn prefix_matches(path: &str, prefix: &str) -> bool {
    if !prefix.contains('{') {
        return path.starts_with(prefix);
    }
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    for part in prefix.split('/').filter(|part| !part.is_empty()) {
        let Some(segment) = segments.next() else {
            return false;
        };
        let placeholder = part.starts_with('{') && part.ends_with('}');
        if !placeholder && segment != part {
            return false;
        }
    }
    !prefix.ends_with('/') || segments.next().is_some()
}

pub async fn filter_path(
    State(routes): State<ComponentRoutes>,
    request: Request,
    next: Next,
) -> Response {
    if matches_path(request.uri().path(), routes) {
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

pub fn with_component_filter(application: Router, routes: ComponentRoutes) -> Router {
    application.layer(from_fn_with_state(routes, filter_path))
}
