use axum::http::HeaderMap;
use axum::http::header::LINK;

pub(crate) fn has_next_link(headers: &HeaderMap) -> bool {
    // httpx joins repeated Link headers with ", " for Headers.get, so the whole value is searched
    let joined = headers
        .get_all(LINK)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect::<Vec<_>>()
        .join(", ");
    joined.contains("rel=\"next\"")
}
