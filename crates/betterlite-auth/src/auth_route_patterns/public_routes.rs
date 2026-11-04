use crate::route_match::templates::route_matches_wildcard_pattern;

#[derive(Debug, thiserror::Error)]
pub enum RoutePatternError {
    #[error("malformed public route settings")]
    MalformedSettings,
}

#[must_use]
pub fn is_public_route(current_route: &str, routes: &[String]) -> bool {
    if routes.iter().any(|route| route == current_route) {
        return true;
    }
    routes
        .iter()
        .any(|pattern| route_matches_wildcard_pattern(current_route, pattern))
}

#[must_use]
pub fn check_configured_public_routes(
    current_route: &str,
    routes: Result<&[String], RoutePatternError>,
) -> bool {
    match routes {
        Ok(configured_routes) => is_public_route(current_route, configured_routes),
        Err(error) => {
            tracing::error!(error = ?error, "failed to read configured public routes");
            false
        }
    }
}
