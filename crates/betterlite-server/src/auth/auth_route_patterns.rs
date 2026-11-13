use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::auth_route_patterns::canonical::{
    RequestRouteSource, ScopeFields, get_request_route,
};
use betterlite_auth::auth_route_patterns::public_routes::check_configured_public_routes;
use betterlite_config::sections::auth_route_patterns::Settings;
use secrecy::SecretString;

use super::route_match::{RouteMatchError, decoded_route_path};

mod tests;

#[derive(Clone)]
pub(crate) struct AuthSettings {
    pub master_key: Option<SecretString>,
    pub routes: Settings,
}

pub(crate) async fn stage(
    State(settings): State<AuthSettings>,
    request: Request,
    next: Next,
) -> Response {
    let Some(path) = decoded_route_path(request.uri().path()) else {
        return RouteMatchError::Unrecognized.into_response();
    };
    // Axum's URI is the dispatched path, including any prefix removal by its enclosing router.
    let route = get_request_route(&RequestRouteSource {
        scope: Ok(Some(ScopeFields {
            path: Some(&path),
            app_root_path: None,
            root_path: None,
        })),
        url_path: &path,
    });
    if check_configured_public_routes(&route, Ok(&settings.routes.public_routes)) {
        next.run(request).await
    } else {
        super::authenticate(State(settings.master_key), request, next).await
    }
}
