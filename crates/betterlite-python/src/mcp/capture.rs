use betterlite_mcp::client::http_transport::capture::{
    BodyChunks, CapturedBody, ErrorCapture, Exchange,
};
use http::HeaderMap;
use std::future::Future;
use std::pin::Pin;
mod tests;
pub(crate) const PREVIEW_CHARS: usize = 512;
const REDACTED: &str = "****";
// httpx's own headers carry nothing secret, and masking them would hide what the server was told
const PUBLIC_HEADERS: &str =
    "content-type content-length accept host user-agent accept-encoding connection";
pub(crate) struct ErrorBodyCapture;
impl ErrorCapture for ErrorBodyCapture {
    // The full body has to survive: a non-2xx answer may hold the JSON-RPC error the client relays
    fn capture<'body>(
        &'body self,
        exchange: Exchange,
        mut body: BodyChunks<'body>,
    ) -> Pin<Box<dyn Future<Output = CapturedBody> + Send + 'body>> {
        Box::pin(async move {
            let mut retained = Vec::new();
            while let Some(chunk) = body.next_chunk().await {
                let Ok(bytes) = chunk else { break };
                retained.extend(bytes);
            }
            let preview = preview(&retained, &secret_values(&exchange.request_headers));
            CapturedBody { retained, preview }
        })
    }
}
fn secret_values(headers: &HeaderMap) -> Vec<String> {
    let mut secrets = Vec::new();
    for (name, value) in headers {
        let public = PUBLIC_HEADERS
            .split(' ')
            .any(|header| header == name.as_str());
        let Ok(text) = value.to_str() else { continue };
        if public || text.is_empty() {
            continue;
        }
        secrets.push(text.to_owned());
        if let Some((_scheme, credential)) = text.split_once(' ') {
            let token = credential.trim();
            if !token.is_empty() && token != text {
                secrets.push(token.to_owned());
            }
        }
    }
    // longest first, so a token is masked whole before its own prefix gets a chance
    secrets.sort_by(|left, right| right.len().cmp(&left.len()));
    secrets.dedup();
    secrets
}
fn preview(body: &[u8], secrets: &[String]) -> String {
    if body.is_empty() {
        return "(empty)".to_owned();
    }
    let Ok(text) = std::str::from_utf8(body) else {
        return "(omitted: body is not UTF-8)".to_owned();
    };
    let mut masked = text.to_owned();
    for secret in secrets {
        masked = masked.replace(secret.as_str(), REDACTED);
    }
    truncate(&masked)
}
fn truncate(text: &str) -> String {
    match text.char_indices().nth(PREVIEW_CHARS) {
        Some((cut, _)) => format!("{}...(truncated)", &text[..cut]),
        None => text.to_owned(),
    }
}
