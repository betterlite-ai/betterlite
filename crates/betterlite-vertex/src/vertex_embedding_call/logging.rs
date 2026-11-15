use url::Url;

use super::post::PostReply;

// LiteLLM's pre_call and post_call logs, cut to facts that carry no header or credential value
pub(super) fn request(url: &str, model: &str, body_bytes: usize) {
    let parsed = Url::parse(url);
    let host = parsed
        .as_ref()
        .ok()
        .and_then(Url::host_str)
        .unwrap_or_default();
    tracing::debug!(
        model,
        host,
        body_bytes,
        "sending the Vertex embedding request"
    );
}

pub(super) fn answer(model: &str, reply: &PostReply) {
    tracing::debug!(
        model,
        status = reply.status,
        "received the Vertex embedding answer"
    );
}
