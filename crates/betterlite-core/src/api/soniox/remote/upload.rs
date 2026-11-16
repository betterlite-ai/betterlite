use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use http::Method;

use super::SonioxRemote;
use super::error::RemoteError;
use super::exchange::{authorization_only, endpoint_url, reply};
use super::transport::{RemoteTransport, Sleeper};
use crate::api::soniox::prepare::decode::identified_resource;
use crate::api::soniox::prepare::raise::raise_for_response;

pub const FILE_FIELD: &str = "file";
pub const FILES_PATH: &str = "/v1/files";
pub const UPLOAD_ACTION: &str = "upload file";

pub async fn upload_file<T, S>(
    remote: &SonioxRemote<'_, T, S>,
    audio: &ProcessedAudioFile,
) -> Result<String, RemoteError>
where
    T: RemoteTransport,
    S: Sleeper,
{
    let file_name = remote
        .prepared
        .options
        .filename_override
        .as_deref()
        .filter(|name| !name.is_empty())
        .unwrap_or(&audio.filename);
    let part = MultipartPart {
        name: FILE_FIELD.to_owned(),
        file_name: Some(file_name.to_owned()),
        content_type: Some(audio.content_type.clone()),
        data: audio.file_content.clone(),
    };
    let request = OutboundRequest {
        method: Method::POST,
        url: endpoint_url(&format!("{}{FILES_PATH}", remote.prepared.api_base))?,
        query: Vec::new(),
        headers: authorization_only(&remote.prepared.auth_headers)?,
        body: RequestBody::Multipart {
            boundary: (remote.boundaries)(),
            parts: vec![part],
        },
        deadline: remote.timeout,
    };
    let response = remote.transport.call(request).await?;
    let decoded = reply(&response, &remote.limits)?;
    raise_for_response(&decoded, UPLOAD_ACTION)?;
    let resource = identified_resource(&decoded)?;
    Ok(resource.id)
}
