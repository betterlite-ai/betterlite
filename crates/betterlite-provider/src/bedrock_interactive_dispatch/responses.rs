use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::pin::Pin;
use std::time::{Duration, SystemTime};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::bedrock_settings::{BedrockError, HeaderError};
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_bedrock::responses::auth::{SignInputs, sign_request, validate_environment};
use betterlite_bedrock::responses::config::BedrockResponsesConfig;
use betterlite_bedrock::responses::error::{ResponsesError, error_from_reply};
use betterlite_bedrock::responses::options::map_openai_params;
use betterlite_bedrock::responses::request::{FetchMode, TransformRequest, transform_responses};
use betterlite_bedrock::responses::url::{CompleteUrlParams, complete_url};
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json;
use betterlite_openai::reasoning_models::DropParams;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::response_params::bundled::BundledReasoningModels;
use betterlite_openai::response_params::capabilities::ResponsesCapabilities;
use betterlite_openai::response_params::params::ResponsesParams;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::value::RawValue;

use super::http::{LeafRequest, LeafTransport};

// Everything the Bedrock Runtime signer and the endpoint need, all of it already resolved upstream
pub struct AwsRequest<'call> {
    pub credentials: Credentials,
    pub api_base: Option<&'call str>,
    pub aws_region_name: Option<&'call str>,
    pub aws_bedrock_runtime_endpoint: Option<&'call str>,
    pub environment_endpoint: Option<&'call str>,
    pub region_inputs: RegionInputs<'call>,
    pub api_key: Option<&'call SecretString>,
    pub environment_token: Option<&'call SecretString>,
    pub headers: IndexMap<String, String>,
    pub signing_params: BTreeMap<String, Box<RawValue>>,
    pub now: SystemTime,
}

// The Responses input, its caller params and the model info the transform reads
pub struct BodyRequest<'call> {
    pub input: &'call ResponsesInput,
    pub params: &'call ResponsesParams,
    pub model_info: Option<&'call Object>,
}

// The native Runtime Responses call: every field is normalized before it arrives, and the credentials are resolved
pub struct Operation<'call, Fetcher, Capabilities> {
    pub aws: AwsRequest<'call>,
    pub body: BodyRequest<'call>,
    pub model: Option<&'call str>,
    pub model_cost: &'call ParamMap,
    pub fetcher: Fetcher,
    pub capabilities: &'call mut Capabilities,
    pub drop_params: DropParams,
    pub bundled: BundledReasoningModels,
    pub deadline: Duration,
}

#[derive(Debug)]
pub struct Outcome {
    pub request: LeafRequest,
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Leaf(#[from] ResponsesError),
    #[error("the model {model} is not served by the Bedrock Runtime Responses API")]
    UnsupportedModel { model: String },
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("the provider rejected the request with status {status}: {message}")]
    Rejected {
        status: u16,
        message: String,
        error: Box<BedrockError>,
    },
    #[error("the Bedrock Runtime Responses reply headers cannot be read")]
    Headers {
        #[source]
        source: HeaderError,
    },
}

pub type RunFuture<'call> = Pin<Box<dyn Future<Output = Result<Outcome, Error>> + Send + 'call>>;

impl<Fetcher, Capabilities> Operation<'_, Fetcher, Capabilities>
where
    Fetcher: RemoteMediaFetcher + Sync + Send,
    Capabilities: ResponsesCapabilities + Send,
{
    pub async fn run(mut self, transport: &dyn LeafTransport) -> Result<Outcome, Error> {
        let Some(model) = self.model else {
            return Err(unsupported(""));
        };
        let url = request_url(&self.aws)?;
        let mapped = map_openai_params(
            self.capabilities,
            model,
            self.body.params,
            self.drop_params,
            &mut self.bundled,
        )
        .await?;
        let Some(config) =
            BedrockResponsesConfig::for_model(Some(model), self.model_cost, self.fetcher)?
        else {
            return Err(unsupported(model));
        };
        let request_data = transform_responses(
            &config.fetcher,
            FetchMode::Each,
            &TransformRequest {
                model,
                input: self.body.input,
                params: &mapped,
                model_info: self.body.model_info,
            },
        )
        .await?;
        let decisions = serde_json::to_value(&request_data).map_err(ResponsesError::Encode)?;
        let request = sign(
            self.aws,
            model,
            &url,
            serde_json::from_value(decisions).map_err(ResponsesError::Encode)?,
            self.deadline,
        )
        .await?;
        deliver(transport, request).await
    }
}

fn unsupported(model: &str) -> Error {
    Error::UnsupportedModel {
        model: model.to_owned(),
    }
}

fn request_url(aws: &AwsRequest<'_>) -> Result<String, Error> {
    complete_url(CompleteUrlParams {
        api_base: aws.api_base,
        aws_region_name: aws.aws_region_name,
        aws_bedrock_runtime_endpoint: aws.aws_bedrock_runtime_endpoint,
        environment_endpoint: aws.environment_endpoint,
        region_inputs: aws.region_inputs,
    })
    .map_err(Error::Leaf)
}

fn header_pairs(headers: &IndexMap<String, String>) -> Vec<(String, Option<String>)> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Some(value.clone())))
        .collect()
}

async fn sign(
    aws: AwsRequest<'_>,
    model: &str,
    url: &str,
    signing_data: IndexMap<String, Json>,
    deadline: Duration,
) -> Result<LeafRequest, Error> {
    let mut headers = aws.headers;
    validate_environment(&mut headers, aws.api_key, aws.environment_token);
    let pairs = header_pairs(&headers);
    let signed = sign_request(
        SignInputs {
            headers: &pairs,
            optional_params: &aws.signing_params,
            request_data: &signing_data,
            api_base: url,
            model: Some(model),
            api_key: aws.api_key,
            environment_token: aws.environment_token,
            region_inputs: aws.region_inputs,
            now: aws.now,
        },
        move |_settings: &AwsSettings, _region: &str| {
            ready(Ok::<Credentials, CredentialError>(aws.credentials))
        },
    )
    .await?;
    let body = signed
        .body
        .unwrap_or_else(|| dumps_object(&signing_data).into_bytes());
    Ok(LeafRequest {
        method: String::from("POST"),
        url: url.to_owned(),
        headers: signed.headers,
        body,
        deadline,
    })
}

// httpx reads the error body as text, so each invalid byte sequence shows as one U+FFFD
fn replaced_text(body: &[u8]) -> String {
    let mut text = String::with_capacity(body.len());
    for chunk in body.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push(char::REPLACEMENT_CHARACTER);
        }
    }
    text
}

async fn deliver(transport: &dyn LeafTransport, request: LeafRequest) -> Result<Outcome, Error> {
    let reply = transport.send(request.clone()).await?;
    if reply.status < 400 {
        return Ok(Outcome {
            request,
            status: reply.status,
            body: reply.body,
        });
    }
    let message = replaced_text(&reply.body);
    let error = error_from_reply(reply.status, &message, &reply.headers)
        .map_err(|source| Error::Headers { source })?;
    Err(Error::Rejected {
        status: reply.status,
        message,
        error: Box::new(error),
    })
}
