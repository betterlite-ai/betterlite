use betterlite_ocr::json::Json;
use betterlite_ocr::request_format::RequestFormat;

#[derive(Debug, Clone, PartialEq)]
pub struct NativeResponse {
    pub payload: Json,
    pub headers: Vec<(String, String)>,
}

// Keeps LiteLLM response headers (cost, call id, etc.) except the ones describing the normalized body.
#[must_use]
pub fn select(
    req_format: Option<RequestFormat>,
    native_payload: Option<&Json>,
    response_headers: &[(String, String)],
) -> Option<NativeResponse> {
    if req_format != Some(RequestFormat::Native) {
        return None;
    }
    let payload = native_payload?.clone();
    let headers = response_headers
        .iter()
        .filter(|(name, _)| {
            !matches!(
                name.to_ascii_lowercase().as_str(),
                "content-length" | "content-type"
            )
        })
        .cloned()
        .collect();
    Some(NativeResponse { payload, headers })
}

mod tests;
