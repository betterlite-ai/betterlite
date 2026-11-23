use std::future::Future;

use betterlite_http::interactions::base::clients::{HttpClients, async_client};
use betterlite_http::interactions::base::{
    ClientError, MAX_RESPONSE_BYTES, Params, RequestFailure,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::limits::read_bounded;

use super::{Answer, DecisionsAsyncTransport, DecisionsRequest};

// adecisions(): get_async_httpx_client(llm_provider=...) and the same POST as a future
impl DecisionsAsyncTransport for HttpClients {
    fn send_async(
        &mut self,
        params: &Params,
        request: DecisionsRequest,
    ) -> Result<impl Future<Output = Answer> + Send, ClientError> {
        let client = async_client(self, params, None)?;
        Ok(async move {
            let mut builder = client
                .post(request.url)
                .headers(request.headers)
                .body(request.body);
            if let Some(timeout) = request.timeout {
                builder = builder.timeout(timeout);
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
    }
}
