use crate::custom_url::get_custom_url;

// get_redirect_url_for_sso: existing_key travels in the state parameter, so the callback URL never carries it
#[must_use]
pub fn get_redirect_url_for_sso(
    proxy_base_url: Option<&str>,
    request_base_url: &str,
    request_root_path: &str,
    sso_callback_route: &str,
    _existing_key: Option<&str>,
) -> String {
    let base = get_custom_url(proxy_base_url, request_base_url, request_root_path, None);
    if base.ends_with('/') {
        format!("{base}{sso_callback_route}")
    } else {
        format!("{base}/{sso_callback_route}")
    }
}
