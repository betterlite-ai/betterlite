use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_models::decisions::Json;
use betterlite_openai::embedding::{EmbeddingInput, EmbeddingResponse};
use secrecy::SecretString;
use serde::Serialize;

use crate::credentials::cache::RefreshProviderCache;
use crate::credentials::source::CredentialSources;
use crate::endpoints::get_client::{ClientCache, GetClientRequest, get_azure_openai_client};
use crate::endpoints::initialize::EndpointError;
use crate::endpoints::projection::{
    CallableProvider, ClientKind, ClientOutcome, ClientSource, LiteLlmParams, SessionKind,
    SuppliedClient,
};

use super::client::{
    ClientFactory, EmbeddingClient, Fields, Headers, ParseError, Parsed, RawEmbedding,
};
use super::error::{AzureOpenAiError, EmbeddingError};
use super::logging::{EmbeddingLogger, PostCallRecord, PreCallRecord, ResponseKind};
use super::reply::{EmbeddingReply, ModelResponse, convert_embedding, response_error};

const DEFAULT_STATUS: u16 = 500;
const TEXT_PREVIEW_CHARACTERS: usize = 500;

pub struct EmbeddingCall<'call, Client> {
    pub model: &'call str,
    pub input: &'call EmbeddingInput,
    pub api_base: &'call str,
    pub api_version: Option<&'call str>,
    pub timeout: f64,
    pub optional_params: Fields,
    pub api_key: Option<SecretString>,
    pub azure_ad_token: Option<SecretString>,
    // accepted for LiteLLM's signature and never forwarded: client settings come from litellm_params
    pub max_retries: Option<i64>,
    pub client: Option<&'call Client>,
    pub aembedding: bool,
    // an empty map is falsy in LiteLLM and leaves optional_params.extra_headers alone
    pub headers: Headers,
    pub litellm_params: LiteLlmParams,
    pub model_response: ModelResponse,
}

pub struct ClientSetup<'data, 'run> {
    pub provider: Option<CallableProvider>,
    pub settings: &'data Settings,
    pub environment: &'data dyn EnvironmentLookup,
    pub refresh_enabled: bool,
    pub session: SessionKind,
    pub sources: &'data dyn CredentialSources,
    pub cache: &'run mut dyn ClientCache,
    pub refresh_cache: &'run mut RefreshProviderCache<'data>,
}

// the call after its pre_call: the logger and the client caches stay out of the future that waits for the SDK
pub struct Begun<'call, Client> {
    call: EmbeddingCall<'call, Client>,
    input: Json,
    data: Fields,
}

fn to_json<T>(value: &T) -> Result<Json, EmbeddingError>
where
    T: Serialize,
{
    serde_json::to_vec(value)
        .and_then(|bytes| serde_json::from_slice(&bytes))
        .map_err(EmbeddingError::Encode)
}

// {model, input, **optional_params}: an option named model or input replaces the value and keeps the position
pub fn begin<'call, Client>(
    call: EmbeddingCall<'call, Client>,
    logger: &mut dyn EmbeddingLogger,
) -> Result<Begun<'call, Client>, EmbeddingError> {
    let input = to_json(call.input).map_err(AzureOpenAiError::wrapped)?;
    let mut optional_params = call.optional_params.clone();
    if !call.headers.is_empty() {
        let headers = call
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect();
        optional_params.insert("extra_headers".to_owned(), Json::Object(headers));
    }
    let mut data = Fields::from([
        ("model".to_owned(), Json::Text(call.model.to_owned())),
        ("input".to_owned(), input.clone()),
    ]);
    data.extend(optional_params);
    logger
        .pre_call(PreCallRecord {
            input: input.clone(),
            api_key: call.api_key.clone(),
            azure_ad_token: call.azure_ad_token.clone(),
            complete_input_dict: data.clone(),
        })
        .map_err(|error| AzureOpenAiError::wrapped(EmbeddingError::Logging(error)))?;
    Ok(Begun { call, input, data })
}

// only the original api_version, model and litellm_params reach the client resolution; max_retries never does
pub fn resolve<'data, Client>(
    begun: &Begun<'_, Client>,
    setup: ClientSetup<'data, '_>,
) -> Result<ClientOutcome<'data>, EndpointError>
where
    Client: EmbeddingClient,
{
    let call = &begun.call;
    let request = GetClientRequest {
        api_key: call.api_key.clone(),
        api_base: Some(call.api_base.to_owned()),
        api_version: call.api_version.map(str::to_owned),
        model: Some(call.model.to_owned()),
        is_async: call.aembedding,
        params: call.litellm_params.clone(),
        provider: setup.provider,
        supplied: call.client.map(|client| SuppliedClient {
            kind: client.kind(),
            custom_query: None,
        }),
        settings: setup.settings,
        environment: setup.environment,
        refresh_enabled: setup.refresh_enabled,
        session: setup.session,
        sources: setup.sources,
    };
    get_azure_openai_client(&request, setup.cache, setup.refresh_cache)
}

fn require_client_family(kind: ClientKind, is_async: bool) -> Result<(), EmbeddingError> {
    match (is_async, kind) {
        (false, ClientKind::AzureOpenAi | ClientKind::OpenAi)
        | (true, ClientKind::AsyncAzureOpenAi | ClientKind::AsyncOpenAi) => Ok(()),
        (true, ClientKind::AzureOpenAi | ClientKind::OpenAi | ClientKind::Other) => {
            Err(EmbeddingError::Value {
                message: "Azure client is not an instance of AsyncAzureOpenAI or AsyncOpenAI"
                    .to_owned(),
            })
        }
        (false, ClientKind::AsyncAzureOpenAi | ClientKind::AsyncOpenAi | ClientKind::Other) => {
            Err(EmbeddingError::Azure(AzureOpenAiError::new(
                DEFAULT_STATUS,
                "azure_client is not an instance of AzureOpenAI or OpenAI",
            )))
        }
    }
}

// both paths check the client's family and send embeddings.with_raw_response.create(**data, timeout=timeout)
pub async fn send<Factory>(
    begun: &Begun<'_, Factory::Client>,
    resolved: Result<ClientOutcome<'_>, EndpointError>,
    factory: &Factory,
) -> Result<RawEmbedding, EmbeddingError>
where
    Factory: ClientFactory,
{
    let call = &begun.call;
    let outcome = resolved?;
    require_client_family(outcome.kind, call.aembedding)?;
    if let Some(supplied) = call.client
        && outcome.source == ClientSource::Supplied
    {
        return Ok(supplied.create(&begun.data, call.timeout).await?);
    }
    let built = factory.client(&outcome)?;
    Ok(built.create(&begun.data, call.timeout).await?)
}

fn unexpected_text(status: u16, text: &str) -> AzureOpenAiError {
    let preview: String = text.chars().take(TEXT_PREVIEW_CHARACTERS).collect();
    AzureOpenAiError::new(
        status,
        format!("Unexpected string response from Azure: {preview}"),
    )
}

fn post_record<Client>(
    begun: &Begun<'_, Client>,
    original_response: Json,
    kind: ResponseKind,
) -> PostCallRecord {
    PostCallRecord {
        input: begun.input.clone(),
        api_key: begun.call.api_key.clone(),
        original_response,
        kind,
        complete_input_dict: begun.data.clone(),
        // only the sync path's successful call names the api_base
        api_base: (kind == ResponseKind::SdkObject).then(|| begun.call.api_base.to_owned()),
    }
}

fn parse_response(
    raw: &RawEmbedding,
    is_async: bool,
) -> Result<Box<EmbeddingResponse>, EmbeddingError> {
    match raw.parse() {
        Ok(Parsed::Embedding(response)) => Ok(response),
        Ok(Parsed::Text(text)) => Err(EmbeddingError::Azure(unexpected_text(raw.status, &text))),
        Err(ParseError::Json(error)) if is_async => {
            let message = format!("Failed to parse raw Azure embedding response: {error}");
            Err(EmbeddingError::Azure(AzureOpenAiError {
                cause: Some(ParseError::Json(error)),
                ..AzureOpenAiError::new(raw.status, message)
            }))
        }
        Err(error) => Err(EmbeddingError::Parse(error)),
    }
}

fn convert<Client>(
    begun: &Begun<'_, Client>,
    raw: RawEmbedding,
    logger: &mut dyn EmbeddingLogger,
) -> Result<Option<EmbeddingReply>, EmbeddingError> {
    let is_async = begun.call.aembedding;
    let response = parse_response(&raw, is_async)?;
    let kind = if is_async {
        ResponseKind::Dumped
    } else {
        ResponseKind::SdkObject
    };
    logger.post_call(post_record(begun, to_json(&*response)?, kind));
    if let Some(error) = response_error(&raw.body) {
        return Err(error);
    }
    if begun.call.model_response == ModelResponse::Other {
        return if is_async {
            Err(EmbeddingError::Azure(AzureOpenAiError::new(
                DEFAULT_STATUS,
                "embedding_response is not an instance of EmbeddingResponse",
            )))
        } else {
            Ok(None)
        };
    }
    Ok(Some(convert_embedding(*response, raw.headers, is_async)))
}

// the sync path wraps every failure in AzureOpenAIError; the async path logs str(error) and raises the original
pub fn finish<Client>(
    begun: &Begun<'_, Client>,
    sent: Result<RawEmbedding, EmbeddingError>,
    logger: &mut dyn EmbeddingLogger,
) -> Result<Option<EmbeddingReply>, EmbeddingError> {
    let result = sent.and_then(|raw| convert(begun, raw, logger));
    if !begun.call.aembedding {
        return result.map_err(|error| EmbeddingError::Azure(AzureOpenAiError::wrapped(error)));
    }
    if let Err(error) = &result {
        let text = Json::Text(error.to_string());
        logger.post_call(post_record(begun, text, ResponseKind::ErrorText));
    }
    result
}
