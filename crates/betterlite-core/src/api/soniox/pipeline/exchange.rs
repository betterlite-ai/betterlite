use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::compatible_wire::header_map;
use betterlite_openai::files::stream::flatten_headers;
use betterlite_openai::openai_base_config::Object;
use http::Method;
use serde_json::to_vec;
use url::Url;

use super::error::PipelineError;
use crate::api::api_boundaries::ApiLimits;
use crate::api::soniox::prepare::decode::{
    IdentifiedResource, SonioxReply, decode_soniox_reply, identified_resource,
};
use crate::api::soniox::prepare::logging::TRANSCRIPTIONS_PATH;
use crate::api::soniox::prepare::raise::raise_for_response;
use crate::api::soniox::prepare::response::transcript;
use crate::api::soniox::remote::SonioxRemote;
use crate::api::soniox::remote::transport::{RemoteTransport, Sleeper};

const CREATE_ACTION: &str = "create transcription";

pub(super) async fn create<T, S>(
    remote: &SonioxRemote<'_, T, S>,
    body: &Object,
) -> Result<IdentifiedResource, PipelineError>
where
    T: RemoteTransport,
    S: Sleeper,
{
    let encoded = to_vec(body).map_err(|source| PipelineError::Body {
        action: CREATE_ACTION,
        source,
    })?;
    let request = OutboundRequest {
        method: Method::POST,
        url: endpoint(remote, TRANSCRIPTIONS_PATH)?,
        query: Vec::new(),
        headers: header_map(&remote.prepared.auth_headers).map_err(PipelineError::Header)?,
        body: RequestBody::Json(encoded),
        deadline: remote.timeout,
    };
    let response = remote.transport.call(request).await?;
    let reply = reply(&response, &remote.limits)?;
    raise_for_response(&reply, CREATE_ACTION)?;
    Ok(identified_resource(&reply)?)
}

pub(super) async fn fetch<T, S>(
    remote: &SonioxRemote<'_, T, S>,
    transcription_id: &str,
) -> Result<Object, PipelineError>
where
    T: RemoteTransport,
    S: Sleeper,
{
    let path = format!("{TRANSCRIPTIONS_PATH}/{transcription_id}/transcript");
    let request = OutboundRequest {
        method: Method::GET,
        url: endpoint(remote, &path)?,
        query: Vec::new(),
        headers: header_map(&remote.prepared.auth_headers).map_err(PipelineError::Header)?,
        body: RequestBody::Empty,
        deadline: remote.timeout,
    };
    let response = remote.transport.call(request).await?;
    let reply = reply(&response, &remote.limits)?;
    raise_for_response(&reply, "fetch transcript")?;
    let fetched = transcript(&reply, &remote.limits)?;
    Ok(fetched.raw)
}

fn endpoint<T, S>(remote: &SonioxRemote<'_, T, S>, path: &str) -> Result<Url, PipelineError>
where
    T: RemoteTransport,
    S: Sleeper,
{
    let base = remote.prepared.api_base.as_str();
    let endpoint = format!("{base}{path}");
    Url::parse(&endpoint).map_err(|source| PipelineError::Url {
        url: endpoint,
        source,
    })
}

fn reply(response: &UpstreamResponse, limits: &ApiLimits) -> Result<SonioxReply, PipelineError> {
    let decoded = decode_soniox_reply(
        i64::from(response.status),
        flatten_headers(&response.headers).into_iter().collect(),
        &response.body,
        limits,
    );
    decoded.map_err(PipelineError::Reply)
}
