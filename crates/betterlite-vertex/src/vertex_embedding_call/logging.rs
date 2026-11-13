use url::Url;

use super::post::PostReply;

pub(super) const TEXT: &str = "text";
pub(super) const MULTIMODAL: &str = "multimodal";

// LiteLLM's pre_call and post_call logs, cut to facts that carry no header or credential value
pub(super) fn request(family: &'static str, url: &str, model: &str, body_bytes: usize) {
    let host = Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_default();
    tracing::debug!(
        family,
        model,
        host = host.as_str(),
        body_bytes,
        "sending the Vertex embedding request"
    );
}

pub(super) fn answer(family: &'static str, model: &str, reply: &PostReply) {
    tracing::debug!(
        family,
        model,
        status = reply.status,
        body_bytes = reply.body.len(),
        "received the Vertex embedding answer"
    );
}
