use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use thiserror::Error;

pub struct PostRequest<'request> {
    pub url: &'request str,
    pub headers: &'request BTreeMap<String, String>,
    pub body: &'request [u8],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostReply {
    pub status: u16,
    pub body: Vec<u8>,
}

pub type PostFuture<'request, Failure> =
    Pin<Box<dyn Future<Output = Result<PostReply, Failure>> + Send + 'request>>;

// LiteLLM's HTTPHandler resolves a None timeout itself, so the argument reaches it every time
pub trait HandlerPost: Send + Sync {
    type Failure: Error + 'static;

    fn post<'request>(
        &'request self,
        request: PostRequest<'request>,
        timeout: Option<Duration>,
    ) -> PostFuture<'request, Self::Failure>;
}

// A raw httpx client keeps its own configured timeout when the caller passed none
pub trait RawClientPost: Send + Sync {
    type Failure: Error + 'static;

    fn post_with_timeout<'request>(
        &'request self,
        request: PostRequest<'request>,
        timeout: Duration,
    ) -> PostFuture<'request, Self::Failure>;

    fn post_with_client_timeout<'request>(
        &'request self,
        request: PostRequest<'request>,
    ) -> PostFuture<'request, Self::Failure>;
}

pub enum CallerClient<'client, Handler, Raw> {
    Handler(&'client Handler),
    Raw(&'client Raw),
}

#[derive(Debug, Error)]
pub enum PostFailure<HandlerFailure, RawFailure, PooledFailure>
where
    HandlerFailure: Error + 'static,
    RawFailure: Error + 'static,
    PooledFailure: Error + 'static,
{
    #[error(transparent)]
    Handler(HandlerFailure),
    #[error(transparent)]
    Raw(RawFailure),
    #[error(transparent)]
    Pooled(PooledFailure),
}

pub async fn post<Handler, Raw, Pooled>(
    caller: Option<CallerClient<'_, Handler, Raw>>,
    pooled: &Pooled,
    request: PostRequest<'_>,
    timeout: Option<Duration>,
) -> Result<PostReply, PostFailure<Handler::Failure, Raw::Failure, Pooled::Failure>>
where
    Handler: HandlerPost,
    Raw: RawClientPost,
    Pooled: HandlerPost,
{
    match caller {
        Some(CallerClient::Handler(client)) => client
            .post(request, timeout)
            .await
            .map_err(PostFailure::Handler),
        Some(CallerClient::Raw(client)) => match timeout {
            Some(given) => client
                .post_with_timeout(request, given)
                .await
                .map_err(PostFailure::Raw),
            None => client
                .post_with_client_timeout(request)
                .await
                .map_err(PostFailure::Raw),
        },
        None => pooled
            .post(request, timeout)
            .await
            .map_err(PostFailure::Pooled),
    }
}
