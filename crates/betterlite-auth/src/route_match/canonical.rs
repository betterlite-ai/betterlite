pub struct ScopeFields<'source> {
    pub path: Option<&'source str>,
    pub app_root_path: Option<&'source str>,
    pub root_path: Option<&'source str>,
}

pub struct RequestRouteSource<'source> {
    pub scope: Option<ScopeFields<'source>>,
    pub url_path: &'source str,
}

// Missing scope models both an unusable ASGI scope and an exception fallback.
#[must_use]
pub fn get_request_route(source: &RequestRouteSource<'_>) -> String {
    let Some(scope) = &source.scope else {
        return source.url_path.to_owned();
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
pub fn is_assistants_api_request(source: &RequestRouteSource<'_>) -> bool {
    let route = get_request_route(source);
    if route.contains("thread") {
        return true;
    }
    route.contains("assistant")
}

#[must_use]
pub fn is_generate_content_route(route: &str) -> bool {
    if route.contains("generateContent") {
        return true;
    }
    route.contains("streamGenerateContent")
}
