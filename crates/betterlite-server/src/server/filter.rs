use axum::extract::{MatchedPath, Request};
use axum::http::StatusCode;
use axum::middleware::Next;
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

// LiteLLM keeps a route when its template is exact, starts with a kept prefix, or is a kept mount
#[must_use]
pub fn keeps_route(template: &str, routes: ComponentRoutes) -> bool {
    routes.exact.contains(template)
        || routes
            .prefixes
            .iter()
            .any(|prefix| template.starts_with(prefix))
        || routes.mounts.iter().any(|mount| {
            template
                .strip_prefix(mount)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        })
}

// the routes arrive as a request extension, so betterlite itself runs this layer with no allowlist and keeps every route
pub async fn filter_route(request: Request, next: Next) -> Response {
    let allowlist = request.extensions().get::<ComponentRoutes>().copied();
    if let (Some(routes), Some(matched)) = (allowlist, request.extensions().get::<MatchedPath>())
        && !keeps_route(matched.as_str(), routes)
    {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(NotFoundBody {
                detail: "Not Found",
            }),
        )
            .into_response();
    }
    next.run(request).await
}
