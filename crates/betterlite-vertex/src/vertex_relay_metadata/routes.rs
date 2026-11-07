use regex::Regex;
use url::Url;

const VERTEX_INTERACTIONS_PATH: &str = "/projects/[^/]+/locations/[^/]+/interactions/?$";

#[must_use]
pub fn is_vertex_interactions_route(url_route: &str) -> bool {
    let Ok(pattern) = Regex::new(VERTEX_INTERACTIONS_PATH) else {
        return false;
    };
    pattern.is_match(&route_path(url_route))
}

#[must_use]
pub fn is_interactions_route(url_route: &str) -> bool {
    route_path(url_route)
        .trim_end_matches('/')
        .ends_with("/interactions")
}

// Python urlparse keeps the raw text up to the query or fragment for a route without a scheme.
fn route_path(url_route: &str) -> String {
    match Url::parse(url_route) {
        Ok(url) => url.path().to_owned(),
        Err(_error) => url_route
            .split(['?', '#'])
            .next()
            .unwrap_or_default()
            .to_owned(),
    }
}
