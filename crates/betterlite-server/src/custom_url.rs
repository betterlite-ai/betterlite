mod tests;

#[must_use]
pub fn join_paths(base_path: &str, route: &str) -> String {
    let base = base_path.trim_end_matches('/');
    let trimmed_route = route.trim_start_matches('/');
    if base.is_empty() {
        return if trimmed_route.is_empty() {
            "/".to_owned()
        } else {
            format!("/{trimmed_route}")
        };
    }
    if trimmed_route.is_empty() {
        return base.to_owned();
    }
    if base
        .strip_suffix(trimmed_route)
        .is_some_and(|prefix| prefix.ends_with('/'))
    {
        return base.to_owned();
    }
    format!("{base}/{trimmed_route}")
}

#[must_use]
pub fn get_custom_url(
    proxy_base_url: Option<&str>,
    request_base_url: &str,
    request_root_path: &str,
    route: Option<&str>,
) -> String {
    let base_url = proxy_base_url.unwrap_or(request_base_url);
    match route {
        Some(route_path) if request_root_path.is_empty() => join_paths(base_url, route_path),
        Some(route_path) => join_paths(&join_paths(base_url, request_root_path), route_path),
        None => join_paths(base_url, request_root_path),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CustomUrls {
    pub proxy_base_url: Option<String>,
}

impl CustomUrls {
    #[must_use]
    pub fn custom_url(&self, request_base_url: &str, request_root_path: &str) -> String {
        join_paths(
            self.proxy_base_url.as_deref().unwrap_or(request_base_url),
            request_root_path,
        )
    }

    #[must_use]
    pub fn custom_route_url(
        &self,
        request_base_url: &str,
        request_root_path: &str,
        route: &str,
    ) -> String {
        get_custom_url(
            self.proxy_base_url.as_deref(),
            request_base_url,
            request_root_path,
            Some(route),
        )
    }
}
