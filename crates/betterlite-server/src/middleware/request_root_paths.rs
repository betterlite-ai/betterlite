pub mod normalize;
mod tests;

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{OriginalUri, State};
use axum::http::uri::{InvalidUri, InvalidUriParts, PathAndQuery};
use axum::http::{Extensions, Method, Request, StatusCode, Uri};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::Response;
use betterlite_config::sections::request_root_paths::Settings;
use thiserror::Error;
use tower::ServiceBuilder;

use self::normalize::normalize_root_paths;
use crate::error_body::json_error;

// A request's effective root path lives in its own extensions, so no request can see another's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestRootPath(pub String);

#[derive(Debug, Error)]
enum RootPathError {
    #[error("the path below the root path is not a valid URI path")]
    Rewrite {
        #[source]
        source: InvalidUri,
    },
    #[error("the rewritten request URI is not valid")]
    Parts {
        #[source]
        source: InvalidUriParts,
    },
}

pub struct RootPaths {
    prefixes: Vec<String>,
    scalar_root: String,
}

impl RootPaths {
    // None when no prefix survives normalization: the middleware is not installed at all then.
    #[must_use]
    pub fn from_settings(settings: &Settings) -> Option<Self> {
        let prefixes = normalize_root_paths(&settings.server_root_paths);
        if prefixes.is_empty() {
            return None;
        }
        let scalar_root = &settings.server_root_path;
        if !scalar_root.is_empty() && scalar_root != "/" {
            tracing::warn!(
                "SERVER_ROOT_PATH and SERVER_ROOT_PATHS are both set ({scalar_root:?}, {prefixes:?}). A request matching a SERVER_ROOT_PATHS prefix overrides the scalar root path for that request; unmatched requests keep SERVER_ROOT_PATH. Configure one mechanism or the other."
            );
        }
        Some(Self {
            prefixes,
            scalar_root: scalar_root.clone(),
        })
    }

    // The service runs before the inner router matches, so the router sees the path without its root.
    pub fn wrap(self, application: Router) -> Router {
        Router::new().fallback_service(
            ServiceBuilder::new()
                .layer(from_fn_with_state(Arc::new(self), assign_root_path))
                .service(application),
        )
    }

    fn effective_root(&self, path: &str) -> &str {
        self.prefixes
            .iter()
            .find(|prefix| {
                path.strip_prefix(prefix.as_str())
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
            })
            .unwrap_or(&self.scalar_root)
    }
}

// The stashed root is the matched prefix, or the scalar root when the middleware ran and nothing matched.
#[must_use]
pub fn get_request_root_path(extensions: &Extensions, scalar_root: &str) -> String {
    extensions
        .get::<RequestRootPath>()
        .map_or_else(|| scalar_root.to_owned(), |stashed| stashed.0.clone())
}

async fn assign_root_path(
    State(paths): State<Arc<RootPaths>>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    if request.method() == Method::CONNECT {
        return next.run(request).await;
    }
    let root = paths.effective_root(request.uri().path()).to_owned();
    let original = request.uri().clone();
    match strip_root(&original, &root) {
        Ok(routed) => *request.uri_mut() = routed,
        Err(error) => {
            tracing::warn!(error = %error, "request URI could not be rewritten below its root path");
            return json_error(
                StatusCode::BAD_REQUEST,
                "request path is not valid",
                "invalid_request_error",
                "400",
            );
        }
    }
    if request.extensions().get::<OriginalUri>().is_none() {
        request.extensions_mut().insert(OriginalUri(original));
    }
    request.extensions_mut().insert(RequestRootPath(root));
    next.run(request).await
}

// Starlette strips root_path only on a whole-segment boundary and routes an exact match as "/".
fn strip_root(uri: &Uri, root: &str) -> Result<Uri, RootPathError> {
    let rest = match uri.path().strip_prefix(root) {
        Some(rest) if !root.is_empty() && (rest.is_empty() || rest.starts_with('/')) => rest,
        _ => return Ok(uri.clone()),
    };
    let path = if rest.is_empty() { "/" } else { rest };
    let path_and_query = uri
        .query()
        .map_or_else(|| path.to_owned(), |query| format!("{path}?{query}"));
    let mut parts = uri.clone().into_parts();
    parts.path_and_query = Some(
        path_and_query
            .parse::<PathAndQuery>()
            .map_err(|source| RootPathError::Rewrite { source })?,
    );
    Uri::from_parts(parts).map_err(|source| RootPathError::Parts { source })
}
