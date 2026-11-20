use std::future::{Future, ready};
use std::pin::Pin;
use std::time::{Duration, SystemTime};

use betterlite_bedrock::agentcore_search_reply::decode_agentcore_search_reply;
use betterlite_bedrock::agentcore_search_reply::error::SearchReplyError;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::search::error::SearchError;
use betterlite_bedrock::search::headers::validate_environment;
use betterlite_bedrock::search::request::transform_search_request;
use betterlite_bedrock::search::signing::{SignRequest, sign_request};
use betterlite_bedrock::search::url::complete_url;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::search::provider::SearchQuery;
use betterlite_openai::search::records::SearchResponse;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::http::{LeafRequest, LeafTransport};

// An AgentCore MCP search: the gateway URL, headers and JSON-RPC body are the leaf's, not this layer's
pub struct Operation<'call> {
    pub query: &'call SearchQuery,
    pub optional_params: &'call IndexMap<String, Json>,
    pub configured_tool_name: Option<&'call str>,
    pub headers: Headers,
    pub configured_protocol_version: Option<&'call str>,
    pub api_base: Option<&'call str>,
    pub configured_gateway_url: Option<&'call str>,
    pub session_region: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub credentials: Credentials,
    pub lookup: &'call (dyn EnvironmentLookup + Sync),
    pub now: SystemTime,
    pub deadline: Duration,
}

#[derive(Debug)]
pub struct Outcome {
    pub request: LeafRequest,
    pub response: SearchResponse,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Leaf(#[from] SearchError),
    #[error(transparent)]
    Reply(#[from] SearchReplyError),
    #[error(transparent)]
    Transport(#[from] TransportError),
}

pub type RunFuture<'call> = Pin<Box<dyn Future<Output = Result<Outcome, Error>> + Send + 'call>>;

// Everything the AgentCore signer reads, captured once the URL, headers and JSON-RPC body exist
struct Signing<'call> {
    headers: Headers,
    optional_params: &'call IndexMap<String, Json>,
    request_data: &'call Json,
    url: &'call str,
    api_key: Option<&'call SecretString>,
    session_region: Option<&'call str>,
    credentials: Credentials,
    lookup: &'call (dyn EnvironmentLookup + Sync),
    now: SystemTime,
    deadline: Duration,
}

impl Operation<'_> {
    pub async fn run(self, transport: &dyn LeafTransport) -> Result<Outcome, Error> {
        let url = complete_url(self.api_base, self.configured_gateway_url)?;
        let api_headers = validate_environment(self.headers, self.configured_protocol_version);
        let request_data =
            transform_search_request(self.query, self.optional_params, self.configured_tool_name)?;
        let signing = Signing {
            headers: api_headers,
            optional_params: self.optional_params,
            request_data: &request_data,
            url: &url,
            api_key: self.api_key,
            session_region: self.session_region,
            credentials: self.credentials,
            lookup: self.lookup,
            now: self.now,
            deadline: self.deadline,
        };
        let request = sign(signing).await?;
        let reply = transport.send(request.clone()).await?;
        let response = decode_agentcore_search_reply(&reply.body, reply.status, &reply.headers)?;
        Ok(Outcome { request, response })
    }
}

async fn sign(signing: Signing<'_>) -> Result<LeafRequest, Error> {
    let Signing {
        headers,
        optional_params,
        request_data,
        url,
        api_key,
        session_region,
        credentials,
        lookup,
        now,
        deadline,
    } = signing;
    let signed = sign_request(
        &SignRequest {
            headers: &headers,
            optional_params,
            request_data,
            api_base: url,
            api_key,
            session_region,
            now,
        },
        lookup,
        move |_settings: &AwsSettings, _region: &str| {
            ready(Ok::<Credentials, CredentialError>(credentials))
        },
    )
    .await?;
    Ok(LeafRequest {
        method: signed.method,
        url: signed.url,
        headers: signed
            .headers
            .into_iter()
            .map(|(name, value)| (name, Some(value)))
            .collect(),
        body: signed.body,
        deadline,
    })
}
