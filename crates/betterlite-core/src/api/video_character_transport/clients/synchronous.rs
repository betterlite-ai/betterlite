use std::io::Read;
use std::sync::Arc;

use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_http::interactions::base::{
    ClientError, MAX_RESPONSE_BYTES, Params, RequestFailure,
};
use betterlite_http::proxy::UpstreamResponse;
use http::header::HeaderMap;

use super::{Answer, CharacterBody, CharacterRequest, SyncSender};

// the blocking sender B173 builds for this call: one request, sent once, its reply read under the shared cap
pub(super) fn sync_sender(
    clients: &mut HttpClients,
    params: &Params,
) -> Result<SyncSender, ClientError> {
    let client = clients.sync_client(params, None)?;
    Ok(SyncSender(Arc::new(
        move |request: CharacterRequest| -> Answer {
            let CharacterRequest {
                method,
                url,
                headers,
                body,
                timeout,
            } = request;
            let mut builder = client.request(method, url).headers(headers);
            if let Some(deadline) = timeout {
                builder = builder.timeout(deadline);
            }
            let outcome = match body {
                CharacterBody::Empty => builder.send(),
                CharacterBody::Multipart(data) => builder.body(data).send(),
            };
            let sent = outcome.map_err(RequestFailure::Transport)?;
            let status = sent.status().as_u16();
            let reply_headers = sent.headers().clone();
            finish_blocking(sent, status, reply_headers)
        },
    )))
}

// the blocking answer, read up to the shared cap so a provider answer cannot grow without bound
fn finish_blocking<S>(sent: S, status: u16, headers: HeaderMap) -> Answer
where
    S: Read,
{
    let cap = u64::try_from(MAX_RESPONSE_BYTES.saturating_add(1)).unwrap_or(u64::MAX);
    let mut body = Vec::new();
    sent.take(cap)
        .read_to_end(&mut body)
        .map_err(RequestFailure::Body)?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(RequestFailure::BodyTooLarge {
            limit: MAX_RESPONSE_BYTES,
        });
    }
    Ok(UpstreamResponse {
        status,
        headers,
        body,
    })
}
