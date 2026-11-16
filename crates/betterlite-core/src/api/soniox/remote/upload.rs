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

impl<T: RemoteTransport, S: Sleeper> SonioxRemote<'_, T, S> {
    pub async fn upload(&self, audio: &ProcessedAudioFile) -> Result<String, RemoteError> {
        let file_name = self
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
            url: endpoint_url(&format!("{}{FILES_PATH}", self.prepared.api_base))?,
            query: Vec::new(),
            headers: authorization_only(&self.prepared.auth_headers)?,
            body: RequestBody::Multipart {
                boundary: (self.boundaries)(),
                parts: vec![part],
            },
            deadline: self.timeout,
        };
        let response = self.transport.call(request).await?;
        let decoded = reply(response, &self.limits)?;
        raise_for_response(&decoded, UPLOAD_ACTION)?;
        let resource = identified_resource(&decoded)?;
        Ok(resource.id)
    }
}
