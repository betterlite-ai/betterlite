use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::compatible_wire::header_map;
use betterlite_openai::files::stream::flatten_headers;
use betterlite_openai::openai_base_config::Object;
use http::Method;
use serde_json::to_vec;
use url::Url;

use super::error::PipelineError;
use crate::api::soniox::prepare::decode::{
    IdentifiedResource, SonioxReply, decode_soniox_reply, identified_resource,
};
use crate::api::soniox::prepare::logging::TRANSCRIPTIONS_PATH;
use crate::api::soniox::prepare::raise::raise_for_response;
use crate::api::soniox::prepare::response::transcript;
use crate::api::soniox::remote::SonioxRemote;
use crate::api::soniox::remote::transport::{RemoteTransport, Sleeper};

const CREATE_ACTION: &str = "create transcription";
const FETCH_ACTION: &str = "fetch transcript";

impl<T, S> SonioxRemote<'_, T, S>
where
    T: RemoteTransport,
    S: Sleeper,
{
    pub(super) async fn create(&self, body: &Object) -> Result<IdentifiedResource, PipelineError> {
        let encoded = to_vec(body).map_err(|source| PipelineError::Body {
            action: CREATE_ACTION,
            source,
        })?;
        let reply = self
            .send(
                CREATE_ACTION,
                Method::POST,
                TRANSCRIPTIONS_PATH,
                RequestBody::Json(encoded),
            )
            .await?;
        Ok(identified_resource(&reply)?)
    }

    pub(super) async fn fetch(&self, transcription_id: &str) -> Result<Object, PipelineError> {
        let path = format!("{TRANSCRIPTIONS_PATH}/{transcription_id}/transcript");
        let reply = self
            .send(FETCH_ACTION, Method::GET, &path, RequestBody::Empty)
            .await?;
        Ok(transcript(&reply, &self.limits)?.raw)
    }

    async fn send(
        &self,
        action: &'static str,
        method: Method,
        path: &str,
        body: RequestBody,
    ) -> Result<SonioxReply, PipelineError> {
        let request = OutboundRequest {
            method,
            url: self.endpoint(path)?,
            query: Vec::new(),
            headers: header_map(&self.prepared.auth_headers).map_err(PipelineError::Header)?,
            body,
            deadline: self.timeout,
        };
        let response = self.transport.call(request).await?;
        let reply = self.reply(&response)?;
        raise_for_response(&reply, action)?;
        Ok(reply)
    }

    fn endpoint(&self, path: &str) -> Result<Url, PipelineError> {
        let base = self.prepared.api_base.as_str();
        let endpoint = format!("{base}{path}");
        Url::parse(&endpoint).map_err(|source| PipelineError::Url {
            url: endpoint,
            source,
        })
    }

    fn reply(&self, response: &UpstreamResponse) -> Result<SonioxReply, PipelineError> {
        let decoded = decode_soniox_reply(
            i64::from(response.status),
            flatten_headers(&response.headers).into_iter().collect(),
            &response.body,
            &self.limits,
        );
        decoded.map_err(PipelineError::Reply)
    }
}
