use std::future::Future;

use betterlite_http::client_kind::{ClientKind, SuppliedClient};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::{HeaderMap, Method};

use crate::anthropic_http_stream::error::{PostFailure, StatusFailure, StreamFailure};
use crate::anthropic_http_stream::transport::{StreamPost, StreamTransport};

pub struct BufferedReply {
    pub status_code: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}
pub trait ChatTransport: StreamTransport {
    fn post_buffered(
        &self,
        post: StreamPost,
    ) -> impl Future<Output = Result<BufferedReply, PostFailure<Self::Failure>>> + Send;
}
impl ChatTransport for ProxiedClient {
    async fn post_buffered(
        &self,
        post: StreamPost,
    ) -> Result<BufferedReply, PostFailure<StreamFailure>> {
        let reply = self
            .send_following(OutboundRequest {
                method: Method::POST,
                url: post.url,
                query: Vec::new(),
                headers: post.headers,
                body: RequestBody::Json(post.body),
                deadline: post.deadline,
            })
            .await
            // the buffered handler wraps every send failure, timeout included, in AnthropicError(status_code=500)
            .map_err(|source| PostFailure::Unknown(StreamFailure::Transport(source)))?;
        let response = reply
            .buffered(self.limits.body_bytes)
            .await
            .map_err(|source| PostFailure::Unknown(StreamFailure::Transport(source)))?;
        if !(200..300).contains(&response.status) {
            return Err(PostFailure::Status(Box::new(StatusFailure {
                status_code: response.status,
                body: response.body,
                exception_headers: None,
                response_headers: Some(response.headers),
            })));
        }
        Ok(BufferedReply {
            status_code: response.status,
            headers: response.headers,
            body: response.body,
        })
    }
}
