mod tests;

use super::base_url::normalize_proxy_base_url;

pub const BYOK_RESOURCE_METADATA_PATH: &str = "/v1/mcp/oauth/protected-resource";
pub const TOKEN_NO_CACHE_HEADERS: [(&str, &str); 2] =
    [("Cache-Control", "no-store"), ("Pragma", "no-cache")];

#[must_use]
pub fn well_known_root_suffix(server_root_path: &str) -> &str {
    if server_root_path == "/" {
        ""
    } else {
        server_root_path
    }
}

#[must_use]
pub fn route_relative_request_path(
    original_path: Option<&str>,
    path: &str,
    app_root_path: Option<&str>,
    root_path: Option<&str>,
) -> String {
    let first = original_path.filter(|value| !value.is_empty());
    let raw_path = first.unwrap_or(path);
    let root = app_root_path
        .filter(|value| !value.is_empty())
        .or_else(|| root_path.filter(|value| !value.is_empty()))
        .unwrap_or_default()
        .trim_end_matches('/');
    if !root.is_empty() && (raw_path == root || raw_path.starts_with(&format!("{root}/"))) {
        return raw_path.get(root.len()..).unwrap_or(raw_path).to_owned();
    }
    raw_path.to_owned()
}

#[must_use]
pub fn passthrough_metadata_url(
    base_url: &str,
    route_path: &str,
    server_name: &str,
    root_suffix: &str,
) -> String {
    let prefix = format!("{base_url}/.well-known/oauth-protected-resource{root_suffix}");
    let tail = if route_path.starts_with(&format!("/{server_name}/mcp")) {
        format!("{server_name}/mcp")
    } else {
        format!("mcp/{server_name}")
    };
    format!("{prefix}/{tail}")
}

#[must_use]
pub fn passthrough_www_authenticate(resource_metadata_url: &str, invalid_token: bool) -> String {
    let error_attr = if invalid_token {
        "error=\"invalid_token\", "
    } else {
        ""
    };
    format!("Bearer {error_attr}resource_metadata=\"{resource_metadata_url}\"")
}

#[must_use]
pub fn byok_www_authenticate(proxy_base_url: &str, request_root_path: &str) -> String {
    let base_url = normalize_proxy_base_url(proxy_base_url)
        .unwrap_or_else(|| request_root_path.trim_end_matches('/').to_owned());
    format!("Bearer resource_metadata=\"{base_url}{BYOK_RESOURCE_METADATA_PATH}\"")
}
