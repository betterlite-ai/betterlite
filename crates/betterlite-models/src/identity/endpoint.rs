use url::Url;

// Anchors the path comparison to a segment boundary, since a naive contains() check is spoofable.
#[must_use]
pub fn endpoint_matches_api_base(endpoint: &str, api_base: &str) -> bool {
    let Some(parsed_endpoint) = parse_with_default_scheme(endpoint) else {
        return false;
    };
    let Some(parsed_api_base) = parse_with_default_scheme(api_base) else {
        return false;
    };

    let endpoint_host = parsed_endpoint
        .host_str()
        .unwrap_or_default()
        .to_lowercase();
    let api_base_host = parsed_api_base
        .host_str()
        .unwrap_or_default()
        .to_lowercase();
    if endpoint_host.is_empty() || endpoint_host != api_base_host {
        return false;
    }

    let endpoint_path = parsed_endpoint.path().trim_end_matches('/');
    if endpoint_path.is_empty() {
        return true;
    }
    let api_base_path = parsed_api_base.path().trim_end_matches('/');
    api_base_path == endpoint_path || api_base_path.starts_with(&format!("{endpoint_path}/"))
}

fn parse_with_default_scheme(value: &str) -> Option<Url> {
    let with_scheme = if value.contains("://") {
        value.to_owned()
    } else {
        format!("https://{value}")
    };
    // A value that still fails to parse with a scheme forced on is treated as a non-match.
    Url::parse(&with_scheme).ok()
}

mod tests;
