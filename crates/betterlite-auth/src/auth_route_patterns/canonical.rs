#[derive(Debug, thiserror::Error)]
pub enum ScopeError {
    #[error("request scope access failed")]
    Access,
}

pub struct ScopeFields<'source> {
    pub path: Option<&'source str>,
    pub app_root_path: Option<&'source str>,
    pub root_path: Option<&'source str>,
}

pub struct RequestRouteSource<'source> {
    pub scope: Result<Option<ScopeFields<'source>>, ScopeError>,
    pub url_path: &'source str,
}

#[must_use]
pub fn get_request_route(source: &RequestRouteSource<'_>) -> String {
    let scope = match &source.scope {
        Ok(Some(fields)) => fields,
        Ok(None) => return source.url_path.to_owned(),
        Err(error) => {
            tracing::debug!(?error, "request route scope failed; using URL path");
            return source.url_path.to_owned();
        }
    };
    let raw_path = scope.path.unwrap_or(source.url_path);
    let root = scope
        .app_root_path
        .or(scope.root_path)
        .unwrap_or("")
        .trim_end_matches('/');
    if root.is_empty() {
        return raw_path.to_owned();
    }
    if raw_path == root {
        return "/".to_owned();
    }
    let prefix = format!("{root}/");
    raw_path
        .strip_prefix(&prefix)
        .map_or_else(|| raw_path.to_owned(), |rest| format!("/{rest}"))
}

#[must_use]
pub fn get_request_route_template(template: Result<Option<&str>, ScopeError>) -> Option<&str> {
    match template {
        Ok(Some(value)) if !value.is_empty() => Some(value),
        Ok(Some(_) | None) => None,
        Err(error) => {
            tracing::debug!(?error, "request route template access failed");
            None
        }
    }
}
