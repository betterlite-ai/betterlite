use std::collections::BTreeMap;
use std::error::Error as StdError;

use betterlite_openai::chat_content::{ChatContent, RefuseDownloads};
use secrecy::ExposeSecret as _;

use crate::gemma_envelope::post::{CallerClient, GemmaPost, PostRequest, post};
use crate::gemma_envelope::request::transform_request;
use crate::gemma_envelope::response::{
    GemmaReply, SingleResponseStreamFactory, handle_fake_stream_response,
};

use super::error::GemmaCallError;
use super::plan::{CallerStyle, GemmaCall, GemmaPlan, SuppliedClient, for_call};
use super::reply::decode_gemma_reply;

const VERTEX_PROVIDER: &str = "vertex_ai";
const ACCEPTED_STATUS: u16 = 200;

pub async fn completion<Client, Pooled, Factory>(
    call: &GemmaCall<'_>,
    supplied: Option<SuppliedClient<'_, Client>>,
    pooled: &Pooled,
    factory: &Factory,
) -> Result<GemmaReply<Factory::Stream>, GemmaCallError<Client::Failure, Pooled::Failure>>
where
    Client: GemmaPost,
    Pooled: GemmaPost,
    Factory: SingleResponseStreamFactory,
{
    let planned = for_call(call, supplied.as_ref().map(|client| client.kind))?;
    let caller = caller_client(&planned, supplied.map(|client| client.client));
    let body = request_body(call, &planned)?;
    let headers = header_values(&planned);
    let reply = post(
        caller,
        pooled,
        PostRequest {
            url: &planned.url,
            headers: &headers,
            body: &body,
        },
        call.timeout,
    )
    .await?;
    if reply.status != ACCEPTED_STATUS {
        return Err(GemmaCallError::Status {
            status: reply.status,
            body: String::from_utf8_lossy(&reply.body).into_owned(),
        });
    }
    let mut response = decode_gemma_reply(&reply.body, call.identifier, call.clock)?;
    response.model.clone_from(&planned.model);
    Ok(handle_fake_stream_response(
        response,
        planned.stream,
        factory,
    ))
}

fn caller_client<'client, Client>(
    planned: &GemmaPlan,
    supplied: Option<&'client Client>,
) -> Option<CallerClient<'client, Client>> {
    match (planned.caller, supplied) {
        (Some(CallerStyle::Handler), Some(client)) => Some(CallerClient::Handler(client)),
        (Some(CallerStyle::Raw), Some(client)) => Some(CallerClient::Raw(client)),
        _ => None,
    }
}

fn request_body<ClientFailure, PooledFailure>(
    call: &GemmaCall<'_>,
    planned: &GemmaPlan,
) -> Result<Vec<u8>, GemmaCallError<ClientFailure, PooledFailure>>
where
    ClientFailure: StdError + 'static,
    PooledFailure: StdError + 'static,
{
    let environment = BTreeMap::new();
    let content = ChatContent {
        model: &planned.model,
        messages: call.messages,
        optional_params: call.optional_params,
        provider: Some(VERTEX_PROVIDER),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment: &environment,
        downloads: &RefuseDownloads,
    };
    serde_json::to_vec(&transform_request(&content)?)
        .map_err(|source| GemmaCallError::Encode { source })
}

fn header_values(planned: &GemmaPlan) -> BTreeMap<String, String> {
    planned
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}
