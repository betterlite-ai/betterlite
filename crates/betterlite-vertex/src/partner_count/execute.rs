use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use url::Url;

use crate::cache_lookup::transport::cache_headers;
use crate::credential_sources::getters::VertexParams;
use crate::endpoint_target::target::{AuthHeader, EndpointTarget};
use crate::vertex_interactions::mint::MintAccessToken;

use super::body::PartnerCountBody;
use super::error::PartnerCountError;
use super::options::PartnerCountOptions;
use super::plan::{build_count_tokens_endpoint, count_tokens_location, strip_version_suffix};
use super::response::{PartnerCount, decode_partner_count};

// LiteLLM posts the count with a 30-second timeout
pub const COUNT_TOKENS_TIMEOUT: Duration = Duration::from_secs(30);

const OK: u16 = 200;

// B173's pooled client in production and a scripted reply in tests
pub trait PartnerCountTransport: Send + Sync {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

impl PartnerCountTransport for ProxiedClient {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        // LiteLLM's httpx handlers follow redirects, so each hop is checked again
        self.send_following(request)
            .await?
            .buffered(self.limits.body_bytes)
            .await
    }
}

pub struct PartnerCountCall<'call> {
    pub model: &'call str,
    pub body: &'call PartnerCountBody,
    pub api_base: Option<&'call str>,
    pub params: VertexParams<'call>,
    pub options: &'call PartnerCountOptions,
    pub settings: &'call VertexSettings,
}

pub async fn handle_count_tokens_request<T>(
    transport: &T,
    call: &PartnerCountCall<'_>,
    mint: &dyn MintAccessToken,
) -> Result<PartnerCount, PartnerCountError>
where
    T: PartnerCountTransport,
{
    let body = call.body.stripped_model();
    if body.messages.is_none() {
        return Err(PartnerCountError::MissingMessages);
    }
    let location = count_tokens_location(call.options, call.params, call.settings);
    let credentials = call.params.safe_get_vertex_ai_credentials(call.settings);
    let project = call.params.safe_get_vertex_ai_project(call.settings);
    let access = mint
        .mint(credentials, project)
        .await
        .map_err(PartnerCountError::Mint)?;
    let model = strip_version_suffix(call.model);
    let endpoint = build_count_tokens_endpoint(model, &access.project, location, call.api_base)?;
    let url = Url::parse(&endpoint).map_err(PartnerCountError::Url)?;
    let target = EndpointTarget {
        url: endpoint,
        auth: Some(AuthHeader::Token(access.token)),
    };
    let headers = cache_headers(&target, &BTreeMap::new()).map_err(PartnerCountError::Header)?;
    let bytes = serde_json::to_vec(&body).map_err(PartnerCountError::Encode)?;
    let request = OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Json(bytes),
        deadline: COUNT_TOKENS_TIMEOUT,
    };
    let reply = transport
        .send(request)
        .await
        .map_err(PartnerCountError::Transport)?;
    decode_reply(reply)
}

// LiteLLM reads the count only from a 200 and puts the raw reply body in the error otherwise
fn decode_reply(reply: UpstreamResponse) -> Result<PartnerCount, PartnerCountError> {
    if reply.status != OK {
        return Err(PartnerCountError::Status {
            status: reply.status,
            body: String::from_utf8(reply.body).unwrap_or_default(),
        });
    }
    decode_partner_count(&reply.body)
}
