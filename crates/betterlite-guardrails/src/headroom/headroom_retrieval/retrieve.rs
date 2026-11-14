use betterlite_http::proxy::limits::{TransportLimits, read_bounded};

use crate::headroom::headroom_compression::HeadroomGuardrail;
use crate::headroom::text::lossy_text;
use crate::primitives::json_value::JsonValue;

const RETRIEVE_PATH: &str = "/v1/retrieve";

pub(crate) async fn fetch(
    guardrail: &HeadroomGuardrail,
    hash: &str,
    query: Option<&str>,
) -> String {
    let Some((status, body)) = send(guardrail, hash, query).await else {
        return format!("[Headroom: retrieval failed for hash={hash}]");
    };
    response_text(hash, status, &body)
}

async fn send(
    guardrail: &HeadroomGuardrail,
    hash: &str,
    query: Option<&str>,
) -> Option<(u16, Vec<u8>)> {
    let headers = match guardrail.settings.request_headers() {
        Ok(headers) => headers,
        Err(error) => {
            tracing::warn!(%error, hash, "Headroom: retrieve request headers are unusable");
            return None;
        }
    };
    let url = format!("{}{RETRIEVE_PATH}/{hash}", guardrail.settings.api_base);
    let mut request = guardrail
        .client
        .get(url)
        .headers(headers)
        .timeout(guardrail.settings.timeout.total);
    if let Some(text) = query {
        request = request.query(&[("query", text)]);
    }
    let mut response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(%error, hash, "Headroom: retrieve failed");
            return None;
        }
    };
    let status = response.status().as_u16();
    let body = match read_bounded(&mut response, TransportLimits::DEFAULT.body_bytes).await {
        Ok(body) => body,
        Err(error) => {
            tracing::warn!(%error, hash, "Headroom: retrieve response is unusable");
            return None;
        }
    };
    Some((status, body))
}

fn response_text(hash: &str, status: u16, bytes: &[u8]) -> String {
    match status {
        404 => format!("[Headroom: hash={hash} not found or expired]"),
        200 => match serde_json::from_slice::<JsonValue>(bytes) {
            Ok(JsonValue::Object(fields)) => match fields.get("original_content") {
                Some(JsonValue::String(content)) => content.clone(),
                _ => lossy_text(bytes),
            },
            _ => lossy_text(bytes),
        },
        other => {
            tracing::warn!(
                status = other,
                hash,
                "Headroom: retrieve returned an error status"
            );
            format!("[Headroom: retrieval error {other} for hash={hash}]")
        }
    }
}
