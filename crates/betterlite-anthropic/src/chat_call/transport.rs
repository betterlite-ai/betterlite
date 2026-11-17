use std::future::Future;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::{HeaderMap, Method};

use crate::anthropic_http_stream::error::{PostFailure, StatusFailure, StreamFailure};
use crate::anthropic_http_stream::transport::{StreamPost, StreamTransport};

// Which handler LiteLLM's isinstance checks accept; a supplied client of the other kind is ignored by that branch
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientKind {
    Sync,
    Async,
}

pub struct SuppliedChatClient<'clients, Client> {
    pub kind: ClientKind,
    pub transport: &'clients Client,
}

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
        let reply = reply
            .buffered(self.limits.body_bytes)
            .await
            .map_err(|source| PostFailure::Unknown(StreamFailure::Transport(source)))?;
        if !(200..300).contains(&reply.status) {
            return Err(PostFailure::Status(Box::new(StatusFailure {
                status_code: reply.status,
                body: reply.body,
                exception_headers: None,
                response_headers: Some(reply.headers),
            })));
        }
        Ok(BufferedReply {
            status_code: reply.status,
            headers: reply.headers,
            body: reply.body,
        })
    }
}
