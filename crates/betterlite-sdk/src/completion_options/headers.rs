use betterlite_models::call_helpers::headers::AdditionalHeaders;

#[must_use]
pub fn merge_request_headers(
    headers: Option<&AdditionalHeaders>,
    extra_headers: Option<&AdditionalHeaders>,
    proxy_headers: &AdditionalHeaders,
) -> AdditionalHeaders {
    // an empty headers map falls through to extra_headers, and extra_headers is applied over the start either way
    let mut merged = headers.cloned().unwrap_or_default();
    if let Some(extra) = extra_headers {
        merged.extend(extra.clone());
    }
    merged.extend(proxy_headers.clone());
    merged
}
