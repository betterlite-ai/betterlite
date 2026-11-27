pub mod gemini;
pub mod openai;
pub mod vertex_http;
pub mod vertex_token;

#[cfg(test)]
mod test_support;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;

// the RAG ports' one exchange: LiteLLM's httpx clients follow redirects, and every reply is buffered under the shared client's cap
pub(crate) async fn exchange(
    client: &ProxiedClient,
    request: OutboundRequest,
) -> Result<UpstreamResponse, TransportError> {
    client
        .send_following(request)
        .await?
        .buffered(client.limits.body_bytes)
        .await
}
