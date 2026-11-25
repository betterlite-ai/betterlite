use axum::http::Uri;
use axum::http::uri::InvalidUri;
use betterlite_anthropic::native_compaction::protocol::CompactionProtocol;

mod tests;

#[must_use]
pub const fn child_path(protocol: CompactionProtocol) -> &'static str {
    match protocol {
        CompactionProtocol::Chat => "/v1/chat/completions",
        CompactionProtocol::Messages => "/v1/messages",
    }
}

pub fn child_uri(
    parent: &Uri,
    root_path: &str,
    protocol: CompactionProtocol,
) -> Result<Uri, InvalidUri> {
    let path = format!(
        "{}{}",
        root_path.trim_end_matches('/'),
        child_path(protocol)
    );
    // an http Uri carries no fragment, and the rebuilt path carries no query, so both are gone
    match (parent.scheme_str(), parent.authority()) {
        (Some(scheme), Some(authority)) => format!("{scheme}://{authority}{path}").parse(),
        _ => path.parse(),
    }
}
