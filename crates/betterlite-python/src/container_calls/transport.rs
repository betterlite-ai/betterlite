use std::future::{Future, ready};
use std::time::Duration;

use betterlite_core::api::container_collection_http::{
    CollectionReply, CollectionRequest, CollectionTransport,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::pool::PoolError;
use betterlite_openai::containers::container_file_http::proxied::{
    outbound_request, reply_headers,
};
use betterlite_openai::containers::container_file_http::request::SslVerify;
use betterlite_openai::containers::container_file_http::transport::{
    AsyncClient, BlockingClient, ClientSource, ContainerHttpReply, ContainerHttpRequest,
    TransferError,
};
use pyo3_async_runtimes::tokio::get_runtime;

mod tests;

#[derive(Clone)]
pub(crate) struct Source {
    pub(crate) client: ProxiedClient,
    pub(crate) deadline: Duration,
}

pub(crate) struct BlockingTransport {
    client: ProxiedClient,
    deadline: Duration,
}

pub(crate) struct AsyncTransport {
    client: ProxiedClient,
    deadline: Duration,
}

impl ClientSource for Source {
    type Blocking = BlockingTransport;
    type Asynchronous = AsyncTransport;

    fn blocking_client(
        &self,
        _ssl_verify: Option<&SslVerify>,
    ) -> Result<BlockingTransport, PoolError> {
        Ok(BlockingTransport {
            client: self.client.clone(),
            deadline: self.deadline,
        })
    }

    fn async_client(&self, _ssl_verify: Option<&SslVerify>) -> Result<AsyncTransport, PoolError> {
        Ok(AsyncTransport {
            client: self.client.clone(),
            deadline: self.deadline,
        })
    }
}

impl BlockingClient for BlockingTransport {
    fn send(&self, request: &ContainerHttpRequest) -> Result<ContainerHttpReply, TransferError> {
        let outbound = outbound_request(request, self.deadline)?;
        get_runtime().block_on(execute(&self.client, outbound))
    }
}

impl CollectionTransport for BlockingTransport {
    fn send(&self, request: &CollectionRequest) -> impl Future<Output = CollectionReply> + Send {
        let deadline = request.timeout.unwrap_or(self.deadline);
        let reply = match collection_outbound(request, deadline) {
            Ok(outbound) => get_runtime().block_on(execute(&self.client, outbound)),
            Err(error) => Err(error),
        };
        ready(reply)
    }
}

impl AsyncClient for AsyncTransport {
    async fn send(
        &self,
        request: &ContainerHttpRequest,
    ) -> Result<ContainerHttpReply, TransferError> {
        let outbound = outbound_request(request, self.deadline)?;
        execute(&self.client, outbound).await
    }
}

impl CollectionTransport for AsyncTransport {
    async fn send(&self, request: &CollectionRequest) -> CollectionReply {
        let deadline = request.timeout.unwrap_or(self.deadline);
        let outbound = collection_outbound(request, deadline)?;
        execute(&self.client, outbound).await
    }
}

// the collection request as the file transport's outbound shape, with the JSON body it carries
fn collection_outbound(
    request: &CollectionRequest,
    deadline: Duration,
) -> Result<OutboundRequest, TransferError> {
    let view = ContainerHttpRequest {
        method: request.method,
        url: request.url.clone(),
        headers: request.headers.clone(),
        query: request.query.clone(),
        file: None,
    };
    let mut outbound = outbound_request(&view, deadline)?;
    if let Some(body) = &request.body {
        outbound.body = RequestBody::Json(body.clone());
    }
    Ok(outbound)
}

async fn execute(
    client: &ProxiedClient,
    outbound: OutboundRequest,
) -> Result<ContainerHttpReply, TransferError> {
    let answer = client
        .send_following(outbound)
        .await?
        .buffered(client.limits.body_bytes)
        .await?;
    Ok(ContainerHttpReply {
        status: answer.status,
        headers: reply_headers(&answer.headers),
        body: answer.body,
    })
}
