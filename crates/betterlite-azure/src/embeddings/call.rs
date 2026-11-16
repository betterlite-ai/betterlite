use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_models::decisions::Json;
use betterlite_openai::embedding::{EmbeddingInput, EmbeddingResponse};
use secrecy::SecretString;

use crate::credentials::cache::RefreshProviderCache;
use crate::credentials::source::CredentialSources;
use crate::endpoints::get_client::{ClientCache, GetClientRequest, get_azure_openai_client};
use crate::endpoints::projection::{
    CallableProvider, ClientKind, ClientOutcome, ClientSource, LiteLlmParams, SessionKind,
    SuppliedClient,
};

use super::client::{
    ClientFactory, EmbeddingClient as _, Fields, Headers, ParseError, Parsed, RawEmbedding,
};
use super::error::{AzureOpenAiError, EmbeddingError};
use super::input::input_json;
use super::logging::{EmbeddingLogger, PostCallRecord, PreCallRecord, ResponseKind};
use super::reply::{EmbeddingReply, ModelResponse, convert_embedding};

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

pub struct ClientContext<'data> {
    pub provider: Option<CallableProvider>,
    pub settings: &'data Settings,
    pub environment: &'data dyn EnvironmentLookup,
    pub refresh_enabled: bool,
    pub session: SessionKind,
    pub sources: &'data dyn CredentialSources,
}

pub struct Dependencies<'data, 'run, Factory> {
    pub factory: &'run Factory,
    pub context: ClientContext<'data>,
    pub cache: &'run mut dyn ClientCache,
    pub refresh_cache: &'run mut RefreshProviderCache<'data>,
    pub logger: &'run mut dyn EmbeddingLogger,
}

struct Prepared {
    input: Json,
    data: Fields,
}

// {model, input, **optional_params}: an option named model or input replaces the value and keeps the position
fn prepare<Client>(
    call: &EmbeddingCall<'_, Client>,
    logger: &mut dyn EmbeddingLogger,
) -> Result<Prepared, EmbeddingError> {
    let input = input_json(call.input);
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
    logger.pre_call(PreCallRecord {
        input: input.clone(),
        api_key: call.api_key.clone(),
        azure_ad_token: call.azure_ad_token.clone(),
        complete_input_dict: data.clone(),
    })?;
    Ok(Prepared { input, data })
}

// only the original api_version, model and litellm_params reach the client resolution; max_retries never does
fn resolve<'data, Factory: ClientFactory>(
    call: &EmbeddingCall<'_, Factory::Client>,
    deps: &mut Dependencies<'data, '_, Factory>,
) -> Result<ClientOutcome<'data>, EmbeddingError> {
    let context = &deps.context;
    let request = GetClientRequest {
        api_key: call.api_key.clone(),
        api_base: Some(call.api_base.to_owned()),
        api_version: call.api_version.map(str::to_owned),
        model: Some(call.model.to_owned()),
        is_async: call.aembedding,
        params: call.litellm_params.clone(),
        provider: context.provider.clone(),
        supplied: call.client.map(|client| SuppliedClient {
            kind: client.kind(),
            custom_query: None,
        }),
        settings: context.settings,
        environment: context.environment,
        refresh_enabled: context.refresh_enabled,
        session: context.session,
        sources: context.sources,
    };
    Ok(get_azure_openai_client(
        &request,
        &mut *deps.cache,
        &mut *deps.refresh_cache,
    )?)
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
            Err(EmbeddingError::Azure(AzureOpenAiError {
                status_code: DEFAULT_STATUS,
                message: "azure_client is not an instance of AzureOpenAI or OpenAI".to_owned(),
                headers: None,
                cause: None,
            }))
        }
    }
}

// both paths resolve the client, check its family and send embeddings.with_raw_response.create(**data, timeout=timeout)
async fn send_request<Factory: ClientFactory>(
    call: &EmbeddingCall<'_, Factory::Client>,
    prepared: &Prepared,
    deps: &mut Dependencies<'_, '_, Factory>,
) -> Result<RawEmbedding, EmbeddingError> {
    let outcome = resolve(call, deps)?;
    require_client_family(outcome.kind, call.aembedding)?;
    if let Some(supplied) = call.client
        && outcome.source == ClientSource::Supplied
    {
        return Ok(supplied.create(&prepared.data, call.timeout).await?);
    }
    let built = deps.factory.client(&outcome)?;
    Ok(built.create(&prepared.data, call.timeout).await?)
}

fn unexpected_text(status: u16, text: &str) -> AzureOpenAiError {
    AzureOpenAiError {
        status_code: status,
        message: format!(
            "Unexpected string response from Azure: {}",
            text.chars()
                .take(TEXT_PREVIEW_CHARACTERS)
                .collect::<String>()
        ),
        headers: None,
        cause: None,
    }
}

fn dumped(response: &EmbeddingResponse) -> Result<Json, EmbeddingError> {
    serde_json::to_vec(response)
        .and_then(|bytes| serde_json::from_slice(&bytes))
        .map_err(EmbeddingError::Encode)
}

fn post_record<Client>(
    call: &EmbeddingCall<'_, Client>,
    prepared: &Prepared,
    original_response: Json,
    kind: ResponseKind,
) -> PostCallRecord {
    PostCallRecord {
        input: prepared.input.clone(),
        api_key: call.api_key.clone(),
        original_response,
        kind,
        complete_input_dict: prepared.data.clone(),
        // only the sync path's successful call names the api_base
        api_base: (kind == ResponseKind::SdkObject).then(|| call.api_base.to_owned()),
    }
}

async fn send_sync<Factory: ClientFactory>(
    call: &EmbeddingCall<'_, Factory::Client>,
    prepared: &Prepared,
    deps: &mut Dependencies<'_, '_, Factory>,
) -> Result<Option<EmbeddingReply>, EmbeddingError> {
    let raw = send_request(call, prepared, deps).await?;
    let response = match raw.parse()? {
        Parsed::Embedding(response) => response,
        Parsed::Text(text) => {
            return Err(EmbeddingError::Azure(unexpected_text(raw.status, &text)));
        }
    };
    let original = dumped(&response)?;
    deps.logger.post_call(post_record(
        call,
        prepared,
        original,
        ResponseKind::SdkObject,
    ));
    if call.model_response == ModelResponse::Other {
        return Ok(None);
    }
    Ok(Some(convert_embedding(*response, raw.headers, false)))
}

async fn send_async<Factory: ClientFactory>(
    call: &EmbeddingCall<'_, Factory::Client>,
    prepared: &Prepared,
    deps: &mut Dependencies<'_, '_, Factory>,
) -> Result<Option<EmbeddingReply>, EmbeddingError> {
    let raw = send_request(call, prepared, deps).await?;
    let response = match raw.parse() {
        Ok(Parsed::Embedding(response)) => response,
        Ok(Parsed::Text(text)) => {
            return Err(EmbeddingError::Azure(unexpected_text(raw.status, &text)));
        }
        Err(ParseError::Json(error)) => {
            return Err(EmbeddingError::Azure(AzureOpenAiError {
                status_code: raw.status,
                message: format!("Failed to parse raw Azure embedding response: {error}"),
                headers: None,
                cause: Some(ParseError::Json(error)),
            }));
        }
        Err(error @ ParseError::Shape(_)) => return Err(EmbeddingError::Parse(error)),
    };
    let original = dumped(&response)?;
    deps.logger
        .post_call(post_record(call, prepared, original, ResponseKind::Dumped));
    if call.model_response == ModelResponse::Other {
        return Err(EmbeddingError::Azure(AzureOpenAiError {
            status_code: DEFAULT_STATUS,
            message: "embedding_response is not an instance of EmbeddingResponse".to_owned(),
            headers: None,
            cause: None,
        }));
    }
    Ok(Some(convert_embedding(*response, raw.headers, true)))
}

// the sync path wraps every failure in AzureOpenAIError; the async path logs str(error) and raises the original
pub async fn embedding<Factory: ClientFactory>(
    call: EmbeddingCall<'_, Factory::Client>,
    deps: &mut Dependencies<'_, '_, Factory>,
) -> Result<Option<EmbeddingReply>, EmbeddingError> {
    let prepared = prepare(&call, &mut *deps.logger).map_err(AzureOpenAiError::wrapped)?;
    if !call.aembedding {
        return send_sync(&call, &prepared, deps)
            .await
            .map_err(|error| EmbeddingError::Azure(AzureOpenAiError::wrapped(error)));
    }
    let result = send_async(&call, &prepared, deps).await;
    if let Err(error) = &result {
        let text = Json::Text(error.to_string());
        deps.logger
            .post_call(post_record(&call, &prepared, text, ResponseKind::ErrorText));
    }
    result
}
