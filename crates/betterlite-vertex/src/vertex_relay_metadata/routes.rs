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

// urlparse moves the last segment's ;parameters out of the path before LiteLLM tests the route.
fn route_path(url_route: &str) -> String {
    let parsed = Url::parse(url_route);
    let path = match &parsed {
        Ok(url) => url.path(),
        Err(_error) => url_route.split(['?', '#']).next().unwrap_or_default(),
    };
    match path.rsplit_once('/') {
        Some((head, segment)) => match segment.split_once(';') {
            Some((last, _parameters)) => format!("{head}/{last}"),
            None => path.to_owned(),
        },
        None => match path.split_once(';') {
            Some((last, _parameters)) => last.to_owned(),
            None => path.to_owned(),
        },
    }
}
