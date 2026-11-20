use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::pin::Pin;
use std::time::{Duration, SystemTime};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_bedrock::vector_stores::config::validate_environment;
use betterlite_bedrock::vector_stores::endpoint::{CompleteUrl, complete_url};
use betterlite_bedrock::vector_stores::error::VectorStoreError;
use betterlite_bedrock::vector_stores::request::{
    SearchOptionalParams, SearchQuery, SearchRequest, build_search_request,
};
use betterlite_bedrock::vector_stores::response::decode_knowledge_base_reply;
use betterlite_bedrock::vector_stores::signing::{SignRequest, sign_request};
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::value::RawValue;

use super::http::{LeafRequest, LeafTransport};

// A knowledge-base retrieve: the filter and userContext the caller supplied reach the leaf body unchanged
pub struct Operation<'call> {
    pub vector_store_id: &'call str,
    pub query: &'call SearchQuery,
    pub optional: &'call SearchOptionalParams,
    pub litellm_params: &'call IndexMap<String, Json>,
    pub extra_body: Option<&'call IndexMap<String, Json>>,
    pub api_base: Option<&'call str>,
    pub configured_endpoint: Option<&'call str>,
    pub environment_endpoint: Option<&'call str>,
    pub aws_region_name: Option<&'call str>,
    pub environment_region_name: Option<&'call str>,
    pub environment_region: Option<&'call str>,
    pub headers: BTreeMap<String, Option<String>>,
    pub signing_params: BTreeMap<String, Box<RawValue>>,
    pub api_key: Option<&'call SecretString>,
    pub configured_bearer: Option<&'call SecretString>,
    pub credentials: Credentials,
    pub region_inputs: RegionInputs<'call>,
    pub now: SystemTime,
    pub deadline: Duration,
}

#[derive(Debug)]
pub struct Outcome {
    pub request: LeafRequest,
    pub response: VectorStoreSearchResponse,
}

#[derive(Debug, thiserror::Error)]
pub enum OperationError {
    #[error(transparent)]
    Leaf(#[from] VectorStoreError),
    #[error(transparent)]
    Transport(#[from] TransportError),
}

pub type RunFuture<'call> =
    Pin<Box<dyn Future<Output = Result<Outcome, OperationError>> + Send + 'call>>;

// Everything the knowledge-base signer reads, captured once the URL, headers and body exist
struct Signing<'call> {
    headers: BTreeMap<String, Option<String>>,
    optional_params: &'call BTreeMap<String, Box<RawValue>>,
    request_data: &'call IndexMap<String, Json>,
    api_base: &'call str,
    api_key: Option<&'call SecretString>,
    configured_bearer: Option<&'call SecretString>,
    credentials: Credentials,
    region_inputs: RegionInputs<'call>,
    now: SystemTime,
    deadline: Duration,
}

impl Operation<'_> {
    pub async fn run(self, transport: &dyn LeafTransport) -> Result<Outcome, OperationError> {
        let base = base_url(&self)?;
        let call = build_search_request(&SearchRequest {
            vector_store_id: self.vector_store_id,
            query: self.query,
            optional: self.optional,
            api_base: &base,
            litellm_params: self.litellm_params,
            extra_body: self.extra_body,
        })?;
        let signing = Signing {
            headers: validate_environment(self.headers),
            optional_params: &self.signing_params,
            request_data: &call.body,
            api_base: &call.url,
            api_key: self.api_key,
            configured_bearer: self.configured_bearer,
            credentials: self.credentials,
            region_inputs: self.region_inputs,
            now: self.now,
            deadline: self.deadline,
        };
        let request = sign(signing).await?;
        let reply = transport.send(request.clone()).await?;
        let response =
            decode_knowledge_base_reply(reply.status, &reply.headers, &reply.body, &call.query)?;
        Ok(Outcome { request, response })
    }
}

fn base_url(operation: &Operation<'_>) -> Result<String, OperationError> {
    complete_url(&CompleteUrl {
        api_base: operation.api_base,
        configured_endpoint: operation.configured_endpoint,
        environment_endpoint: operation.environment_endpoint,
        aws_region_name: operation.aws_region_name,
        environment_region_name: operation.environment_region_name,
        environment_region: operation.environment_region,
    })
    .map_err(OperationError::Leaf)
}

async fn sign(signing: Signing<'_>) -> Result<LeafRequest, OperationError> {
    let pairs: Vec<(String, Option<String>)> = signing.headers.into_iter().collect();
    let signed = sign_request(
        SignRequest {
            headers: &pairs,
            optional_params: signing.optional_params,
            request_data: signing.request_data,
            api_base: signing.api_base,
            api_key: signing.api_key,
            configured_bearer: signing.configured_bearer,
            region_inputs: signing.region_inputs,
            now: signing.now,
        },
        move |_settings: &AwsSettings, _region: &str| {
            ready(Ok::<Credentials, CredentialError>(signing.credentials))
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
        deadline: signing.deadline,
    })
}
