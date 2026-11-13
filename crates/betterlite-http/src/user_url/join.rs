use crate::user_url::error::{UrlParseError, UserUrlError};
use crate::user_url::httpx_url::HttpxUrl;
use crate::user_url::split::{USES_NETLOC, urlparse};

// Python 3.13 urllib.parse.uses_relative
const USES_RELATIVE: [&str; 20] = [
    "", "ftp", "http", "gopher", "nntp", "imap", "wais", "file", "https", "shttp", "mms",
    "prospero", "rtsp", "rtsps", "rtspu", "sftp", "svn", "svn+ssh", "ws", "wss",
];

fn resolve_path(base_path: &str, path: &str) -> String {
    let mut base_segments: Vec<&str> = base_path.split('/').collect();
    if base_segments.last() != Some(&"") {
        base_segments.pop();
    }
    let segments: Vec<&str> = if path.starts_with('/') {
        path.split('/').collect()
    } else {
        let joined: Vec<&str> = base_segments.into_iter().chain(path.split('/')).collect();
        let last = joined.len().saturating_sub(1);
        joined
            .into_iter()
            .enumerate()
            .filter(|(index, segment)| *index == 0 || *index == last || !segment.is_empty())
            .map(|(_, segment)| segment)
            .collect()
    };
    let mut resolved: Vec<&str> = Vec::new();
    for segment in &segments {
        match *segment {
            ".." => {
                resolved.pop();
            }
            "." => {}
            other => resolved.push(other),
        }
    }
    if matches!(segments.last(), Some(&("." | ".."))) {
        resolved.push("");
    }
    let joined = resolved.join("/");
    if joined.is_empty() {
        "/".to_owned()
    } else {
        joined
    }
}

// Python 3.13 urllib.parse.urljoin
pub fn urljoin(base: &str, url: &str) -> Result<String, UrlParseError> {
    if base.is_empty() {
        return Ok(url.to_owned());
    }
    if url.is_empty() {
        return Ok(base.to_owned());
    }
    let base_parts = urlparse(base, "")?;
    let mut target = urlparse(url, &base_parts.scheme)?;
    if target.scheme != base_parts.scheme || !USES_RELATIVE.contains(&target.scheme.as_str()) {
        return Ok(url.to_owned());
    }
    if USES_NETLOC.contains(&target.scheme.as_str()) {
        if !target.netloc.is_empty() {
            return Ok(target.unparse());
        }
        target.netloc = base_parts.netloc;
    }
    if target.path.is_empty() && target.params.is_empty() {
        target.path = base_parts.path;
        target.params = base_parts.params;
        if target.query.is_empty() {
            target.query = base_parts.query;
        }
        return Ok(target.unparse());
    }
    target.path = resolve_path(&base_parts.path, &target.path);
    Ok(target.unparse())
}

// httpx 0.28.1 URL(base).join(location)
pub fn join(base: &HttpxUrl, location: &str) -> Result<HttpxUrl, UserUrlError> {
    let reference = HttpxUrl::parse(location)?;
    let joined = urljoin(&base.href(), &reference.href())?;
    Ok(HttpxUrl::parse(&joined)?)
}
