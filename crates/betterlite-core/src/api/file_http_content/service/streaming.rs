use std::num::NonZeroUsize;

use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::files::stream::{FileContentStreamingResult, read_error_body};
use http::Method;

use super::super::config::{FileContentConfig, StreamContext};
use super::super::error::ContentError;
use super::{
    ClientSource, ContentRequest, ContentTransport, PreparedContent, decoded_body_headers, facts,
    query,
};

// async_retrieve_file_content_streaming: the prepared query overrides the transform's own, and the body stays unread until the caller iterates
pub async fn retrieve_content_streaming<C, S>(
    prepared: PreparedContent<'_, C, S>,
    chunk_size: NonZeroUsize,
    logging: &mut LoggingState,
) -> Result<FileContentStreamingResult, ContentError>
where
    C: FileContentConfig,
    S: ClientSource<Client: ContentTransport>,
{
    let request = ContentRequest {
        method: Method::GET,
        url: query::streaming_url(&prepared.url, &prepared.query),
        query: Vec::new(),
        headers: prepared.headers,
        timeout: prepared.timeout,
    };
    let reply = prepared
        .client
        .send_stream(request)
        .await
        .map_err(|error| ContentError::Provider(facts::transport_error(prepared.config, &error)))?;
    if reply.status >= 400 {
        let error_body = read_error_body(reply.body).await;
        return Err(ContentError::Provider(facts::status_error(
            prepared.config,
            reply.status,
            &reply.headers,
            &error_body,
        )));
    }
    let decoded = decoded_body_headers(&reply.headers);
    let context = StreamContext {
        request_url: reply.url.as_str(),
        logging,
        params: &prepared.params,
    };
    Ok(prepared
        .config
        .transform_content_stream(reply.body, &decoded, chunk_size, context))
}
