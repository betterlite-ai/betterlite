use url::Url;

// LiteLLM's pre_call and post_call logs, cut to facts that carry no header or credential value
pub(super) fn request(url: &str, model: &str, body_bytes: usize) {
    let parsed = Url::parse(url);
    let host = parsed
        .as_ref()
        .ok()
        .and_then(Url::host_str)
        .unwrap_or_default();
    tracing::debug!(model, host, body_bytes, "sending the Gemini request");
}
