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

// The handler takes the timeout argument every time; a raw client leaves it out when the caller passed none
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timeout {
    Explicit(Option<Duration>),
    ClientDefault,
}

pub type PostFuture<'request, Failure> =
    Pin<Box<dyn Future<Output = Result<PostReply, Failure>> + Send + 'request>>;

pub trait GemmaPost: Send + Sync {
    type Failure: Error + 'static;

    fn post<'request>(
        &'request self,
        request: PostRequest<'request>,
        timeout: Timeout,
    ) -> PostFuture<'request, Self::Failure>;
}

pub enum CallerClient<'client, Client> {
    Handler(&'client Client),
    Raw(&'client Client),
}

#[derive(Debug, Error)]
pub enum PostFailure<ClientFailure, PooledFailure>
where
    ClientFailure: Error + 'static,
    PooledFailure: Error + 'static,
{
    #[error(transparent)]
    Client(ClientFailure),
    #[error(transparent)]
    Pooled(PooledFailure),
}

pub async fn post<Client, Pooled>(
    caller: Option<CallerClient<'_, Client>>,
    pooled: &Pooled,
    request: PostRequest<'_>,
    timeout: Option<Duration>,
) -> Result<PostReply, PostFailure<Client::Failure, Pooled::Failure>>
where
    Client: GemmaPost,
    Pooled: GemmaPost,
{
    match caller {
        Some(CallerClient::Handler(client)) => client
            .post(request, Timeout::Explicit(timeout))
            .await
            .map_err(PostFailure::Client),
        Some(CallerClient::Raw(client)) => {
            let sent = if timeout.is_some() {
                Timeout::Explicit(timeout)
            } else {
                Timeout::ClientDefault
            };
            client
                .post(request, sent)
                .await
                .map_err(PostFailure::Client)
        }
        None => pooled
            .post(request, Timeout::Explicit(timeout))
            .await
            .map_err(PostFailure::Pooled),
    }
}
