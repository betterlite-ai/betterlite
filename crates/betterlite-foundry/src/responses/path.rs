use betterlite_http::user_url::error::InvalidUrl;
use betterlite_http::user_url::httpx_url::HttpxUrl;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PathError {
    #[error("the URL path cannot be rewritten because it holds a '?' or '#'")]
    Unrewritable,
    #[error(transparent)]
    InvalidUrl(#[from] InvalidUrl),
}

// httpx's copy_with(path=...) parses the decoded path again, so quoting and dot-segment removal come from HttpxUrl::parse
pub(crate) fn requoted_path(base: &HttpxUrl, path: &str) -> Result<String, PathError> {
    if path.contains(['?', '#']) {
        return Err(PathError::Unrewritable);
    }
    let mut authority = base.clone();
    authority.path = String::new();
    authority.query = None;
    authority.fragment = None;
    HttpxUrl::parse(&format!("{}{path}", authority.href()))
        .map(|composed| composed.path)
        .map_err(PathError::InvalidUrl)
}
