use std::sync::Arc;

use betterlite_http::interactions::base::clients::{HttpClients, async_client};
use betterlite_http::interactions::base::{
    ClientError, MAX_RESPONSE_BYTES, Params, RequestFailure,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::limits::read_bounded;

use super::{AsyncAnswer, AsyncSender, CharacterBody, CharacterRequest};

// the async sender B173 builds for this call: the same request sent as a future, read under the same cap
pub(super) fn async_sender(
    clients: &mut HttpClients,
    params: &Params,
) -> Result<AsyncSender, ClientError> {
    let client = async_client(clients, params, None)?;
    Ok(AsyncSender(Arc::new(
        move |request: CharacterRequest| -> AsyncAnswer {
            let cloned = client.clone();
            Box::pin(async move {
                let mut builder = cloned
                    .request(request.method, request.url)
                    .headers(request.headers);
                if let Some(timeout) = request.timeout {
                    builder = builder.timeout(timeout);
                }
                if let CharacterBody::Multipart(data) = request.body {
                    builder = builder.body(data);
                }
                let mut sent = builder.send().await.map_err(RequestFailure::Transport)?;
                let status = sent.status().as_u16();
                let headers = sent.headers().clone();
                let body = read_bounded(&mut sent, MAX_RESPONSE_BYTES)
                    .await
                    .map_err(RequestFailure::from)?;
                Ok(UpstreamResponse {
                    status,
                    headers,
                    body,
                })
            })
        },
    )))
}
