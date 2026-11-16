use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credentials::cache::RefreshProviderCache;
use crate::credentials::headers::{ClientAuthParams, get_azure_request_auth_headers};
use crate::credentials::source::{CallableTokenProvider, CredentialSources, TokenReply};
use crate::endpoints::initialize::{InitializeRequest, initialize_azure_sdk_client};
use crate::endpoints::projection::{
    AzureClientInputs, CallableProvider, SessionKind, TokenProvider,
};

use super::error::{ClassifiedError, ImageError};
use super::generation::Generation;
use super::wire::text_value;

pub(super) struct PrepareContext<'data> {
    pub(super) endpoints: &'data EndpointSettings,
    pub(super) environment: &'data dyn EnvironmentLookup,
    pub(super) sources: &'data dyn CredentialSources,
    pub(super) refresh_enabled: bool,
}

// What LiteLLM resolves before the async leg takes over: the body, the auth headers and the send inputs
pub(super) struct Prepared {
    pub(super) data: IndexMap<String, Json>,
    // the prompt argument, which only the sync post_call reads
    pub(super) prompt: String,
    // the prompt as the body carries it, which pre_call reads
    pub(super) body_prompt: String,
    // the client's endpoint fields, which the URL is built from after the async leg takes over
    pub(super) azure_endpoint: Option<String>,
    pub(super) base_url: Option<String>,
    pub(super) headers: Vec<(String, String)>,
    pub(super) resolved_version: String,
    pub(super) argument_version: String,
    pub(super) deployment: Option<String>,
    pub(super) timeout: Option<f64>,
    pub(super) hidden_model: Option<String>,
    pub(super) async_mode: bool,
    pub(super) now_seconds: i64,
}

pub(super) fn prepare(
    mut call: Generation<'_>,
    context: &PrepareContext<'_>,
) -> Result<Prepared, ImageError> {
    let mut optional_params = std::mem::take(&mut call.optional_params);
    let model = call.model.filter(|name| !name.is_empty());
    let data = request_data(&call, model, &mut optional_params)?;
    let inputs = client_inputs(&call, model, context)?;
    let headers = request_headers(&call, &inputs)?;
    Ok(Prepared {
        prompt: call.prompt.to_owned(),
        body_prompt: text_value(&data, "prompt")?.unwrap_or_default(),
        azure_endpoint: inputs.endpoint.azure_endpoint.clone(),
        base_url: inputs.endpoint.base_url.clone(),
        resolved_version: inputs.api_version.clone(),
        argument_version: call.api_version.unwrap_or_default().to_owned(),
        deployment: model.map(str::to_owned),
        hidden_model: call.base_model.map(str::to_owned),
        data,
        headers,
        timeout: call.timeout,
        async_mode: call.delegate_async,
        now_seconds: call.now_seconds,
    })
}

// extra_body is not an Azure parameter: its keys flatten over the rest of the request body
fn request_data(
    call: &Generation<'_>,
    model: Option<&str>,
    optional_params: &mut IndexMap<String, Json>,
) -> Result<IndexMap<String, Json>, ImageError> {
    let extra = match optional_params.shift_remove("extra_body") {
        Some(Json::Object(fields)) => fields,
        None => IndexMap::new(),
        // an explicit null is unpacked as a mapping too, and LiteLLM fails it with a TypeError
        Some(_) => return Err(ImageError::ExtraBody),
    };
    let body_model = call.base_model.filter(|name| !name.is_empty()).or(model);
    let mut data = IndexMap::new();
    let model_value = body_model.map_or(Json::Null, |name| Json::Text(name.to_owned()));
    data.insert("model".to_owned(), model_value);
    data.insert("prompt".to_owned(), Json::Text(call.prompt.to_owned()));
    data.extend(optional_params.clone());
    data.extend(extra);
    match data.shift_remove("max_retries") {
        None | Some(Json::Bool(_)) => {}
        Some(Json::Number(number)) if number.is_i64() || number.is_u64() => {}
        Some(_) => return Err(ImageError::MaxRetries),
    }
    Ok(data)
}

fn client_inputs<'data>(
    call: &Generation<'_>,
    model: Option<&str>,
    context: &PrepareContext<'data>,
) -> Result<AzureClientInputs<'data>, ImageError> {
    let mut params = call.params.clone();
    if let Some(token) = call.azure_ad_token {
        params.azure_ad_token = Some(token.clone());
    }
    // The image call never builds an SDK client, so the identity B531 reads off id() is unused here
    let provider = call
        .azure_ad_token_provider
        .clone()
        .map(|provider| CallableProvider {
            provider,
            identity: 0,
        });
    let mut refresh = RefreshProviderCache::new(context.sources);
    initialize_azure_sdk_client(
        &InitializeRequest {
            params,
            provider,
            api_key: call.api_key.cloned(),
            api_base: call.api_base.map(str::to_owned),
            model_name: Some(model.unwrap_or_default().to_owned()),
            api_version: call.api_version.map(str::to_owned),
            is_async: false,
            session: SessionKind::Shared,
            refresh_enabled: context.refresh_enabled,
            settings: context.endpoints,
            environment: context.environment,
            sources: context.sources,
        },
        &mut refresh,
    )
    .map_err(ImageError::from)
}

fn request_headers(
    call: &Generation<'_>,
    inputs: &AzureClientInputs<'_>,
) -> Result<Vec<(String, String)>, ImageError> {
    let token = resolve_token(
        inputs.azure_ad_token.clone(),
        inputs.token_provider.as_ref(),
    )?;
    let api_key = inputs
        .api_key
        .as_ref()
        .map(|key| key.expose_secret().to_owned());
    get_azure_request_auth_headers(
        &call.headers,
        &ClientAuthParams {
            azure_ad_token: token,
            azure_ad_token_provider: None,
            api_key,
        },
    )
    .map_err(|failure| ImageError::Classified(ClassifiedError::from(failure)))
}

// B529 resolves the same way, but it takes the provider as a 'static Arc while B531 hands back a borrowed one
fn resolve_token(
    token: Option<SecretString>,
    provider: Option<&TokenProvider<'_>>,
) -> Result<Option<SecretString>, ClassifiedError> {
    if let Some(token) = token.filter(|value| !value.expose_secret().is_empty()) {
        return Ok(Some(token));
    }
    let Some(provider) = provider else {
        return Ok(None);
    };
    match provider.0.token().map_err(ClassifiedError::from)? {
        TokenReply::Text(token) if !token.expose_secret().is_empty() => Ok(Some(token)),
        TokenReply::Text(_) | TokenReply::NonText { .. } => Ok(None),
    }
}
