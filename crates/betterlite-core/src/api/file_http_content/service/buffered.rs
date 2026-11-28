use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::files::records::BinaryContent;
use http::Method;

use super::super::config::FileContentConfig;
use super::super::error::ContentError;
use super::{ClientSource, ContentRequest, ContentTransport, PreparedContent, facts};

// retrieve_file_content and async_retrieve_file_content share this GET: no per-call timeout, and a status of 400 or more raises the provider's error
pub async fn retrieve_file_content<C, S>(
    prepared: PreparedContent<'_, C, S>,
    logging: &mut LoggingState,
) -> Result<BinaryContent, ContentError>
where
    C: FileContentConfig,
    S: ClientSource<Client: ContentTransport>,
{
    let PreparedContent {
        config,
        client,
        url,
        query,
        headers,
        params,
        ..
    } = prepared;
    let pairs: Vec<(String, String)> = query.into_iter().collect();
    let reply = client
        .send(ContentRequest {
            method: Method::GET,
            url,
            query: pairs,
            headers,
            timeout: None,
        })
        .await
        .map_err(|error| ContentError::Provider(facts::transport_error(config, &error)))?;
    if reply.status >= 400 {
        return Err(ContentError::Provider(facts::status_error(
            config,
            reply.status,
            &reply.headers,
            &reply.body,
        )));
    }
    config.transform_content_response(&facts::buffered_response(reply), logging, &params)
}
