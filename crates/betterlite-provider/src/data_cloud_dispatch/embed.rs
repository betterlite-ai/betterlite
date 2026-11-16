use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_openai::compatible_embedding::request_body;
use betterlite_openai::compatible_wire::header_map;
use betterlite_openai::embedding::{EmbeddingInput, EmbeddingResponse};
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use http::Method;
use secrecy::ExposeSecret as _;

use crate::agent_platform_dispatch::context::headers_or_global;
use crate::databricks_auth::oauth::OauthTokenSource;
use crate::databricks_embedding::{EmbeddingPreparation, prepare_embedding};
use crate::oci_common::Clock;
use crate::oci_common::credentials::{OciCredentials, resolve_oci_credentials};
use crate::oci_common::environment::OciEnvironment;
use crate::oci_common::signing::sign_oci_request;
use crate::snowflake::config::{
    Endpoint as SnowflakeEndpoint, get_complete_url, validate_environment,
};
use crate::snowflake::embedding::{
    get_error_class, transform_embedding_request, transform_embedding_response,
};

use super::call::{EmbeddingCall, EmbeddingReply};
use super::context::{
    DispatchGlobals, databricks_environment, environment_secret, environment_text, oci_environment,
    oci_explicit_params,
};
use super::error::DispatchError;
use super::logging::convert;
use super::select::EmbeddingConfig;
use super::transport::{PostTransport, WireRequest};

// litellm/custom_httpx/http_handler.py's default request timeout
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

pub struct EmbeddingContext<'request> {
    pub globals: DispatchGlobals<'request>,
    pub secrets: &'request SecretLookup,
    pub environment: &'request BTreeMap<String, String>,
    pub transport: &'request dyn PostTransport,
    pub tokens: &'request dyn OauthTokenSource,
    pub clock: &'request dyn Clock,
}

pub async fn dispatch(
    call: &EmbeddingCall,
    config: EmbeddingConfig,
    context: &EmbeddingContext<'_>,
) -> Result<EmbeddingReply, DispatchError> {
    match config {
        EmbeddingConfig::Databricks => databricks(call, context).await,
        EmbeddingConfig::Snowflake => snowflake(call, context).await,
        EmbeddingConfig::Oci => oci(call, context).await,
    }
}

// The databricks arm resolves base and key like _complete_databricks, then B5980's prepare_embedding
async fn databricks(
    call: &EmbeddingCall,
    context: &EmbeddingContext<'_>,
) -> Result<EmbeddingReply, DispatchError> {
    let mut api_base = call.api_base.clone().filter(|value| !value.is_empty());
    if api_base.is_none() {
        api_base = context.globals.api_base.map(str::to_owned);
    }
    if api_base.is_none() {
        api_base = environment_text(context.secrets, context.environment, "DATABRICKS_API_BASE")?;
    }
    let mut api_key = call
        .api_key
        .as_ref()
        .filter(|value| !value.expose_secret().is_empty())
        .cloned();
    if api_key.is_none() {
        api_key = context
            .globals
            .api_key
            .filter(|value| !value.expose_secret().is_empty())
            .cloned();
    }
    if api_key.is_none() {
        api_key = context
            .globals
            .databricks_key
            .filter(|value| !value.expose_secret().is_empty())
            .cloned();
    }
    if api_key.is_none() {
        api_key = environment_secret(context.secrets, context.environment, "DATABRICKS_API_KEY")?;
    }
    let environment = databricks_environment(context.secrets, context.environment)?;
    let prepared = prepare_embedding(
        EmbeddingPreparation {
            optional_params: call.optional_params.clone(),
            api_key: api_key.as_ref(),
            api_base: api_base.as_deref(),
            custom_endpoint: false,
            headers: headers_or_global(call.headers.clone(), context.globals.headers),
        },
        &environment,
        context.tokens,
    )
    .await?;
    let data = request_body(
        call.model.clone(),
        &convert(&call.input)?,
        convert(&prepared.optional_params)?,
    );
    let body = serde_json::to_vec(&data).map_err(|source| DispatchError::Encode { source })?;
    let headers =
        header_map(&prepared.headers).map_err(|source| DispatchError::Headers { source })?;
    let response = context
        .transport
        .post(&WireRequest {
            method: Method::POST,
            url: &prepared.url,
            headers: &headers,
            body: &body,
            deadline: call.timeout.unwrap_or(DEFAULT_TIMEOUT),
        })
        .await?;
    if !(200..300).contains(&response.status) {
        return Err(DispatchError::Upstream {
            status: response.status,
            body: String::from_utf8_lossy(&response.body).into_owned(),
        });
    }
    let reply: EmbeddingResponse = serde_json::from_slice(&response.body)
        .map_err(|source| DispatchError::Decode { source })?;
    Ok(EmbeddingReply::Databricks(Box::new(reply)))
}

// The snowflake arm takes the key from the caller or SNOWFLAKE_JWT and calls B5844's embedding config
async fn snowflake(
    call: &EmbeddingCall,
    context: &EmbeddingContext<'_>,
) -> Result<EmbeddingReply, DispatchError> {
    let mut api_key = call
        .api_key
        .as_ref()
        .filter(|value| !value.expose_secret().is_empty())
        .cloned();
    if api_key.is_none() {
        api_key = environment_secret(context.secrets, context.environment, "SNOWFLAKE_JWT")?;
    }
    let account_id =
        environment_text(context.secrets, context.environment, "SNOWFLAKE_ACCOUNT_ID")?;
    let mut optional_params: Object = call.optional_params.clone();
    let url = get_complete_url(
        call.api_base.as_deref(),
        &SnowflakeEndpoint::Embedding,
        &mut optional_params,
        account_id.as_deref(),
    )?;
    let mut headers: Headers = call.headers.clone().unwrap_or_default();
    validate_environment(
        &mut headers,
        &call.model,
        api_key.as_ref().map(|value| value.expose_secret().as_str()),
    )?;
    let body = transform_embedding_request(&call.input, &call.model, &optional_params);
    let bytes = serde_json::to_vec(&body).map_err(|source| DispatchError::Encode { source })?;
    let header_map = header_map(&headers).map_err(|source| DispatchError::Headers { source })?;
    let response = context
        .transport
        .post(&WireRequest {
            method: Method::POST,
            url: &url,
            headers: &header_map,
            body: &bytes,
            deadline: call.timeout.unwrap_or(DEFAULT_TIMEOUT),
        })
        .await?;
    if !(200..300).contains(&response.status) {
        return Err(DispatchError::SnowflakeException(get_error_class(
            &String::from_utf8_lossy(&response.body),
            response.status,
            Headers::new(),
        )));
    }
    let reply =
        transform_embedding_response(&call.model, &String::from_utf8_lossy(&response.body))?;
    Ok(EmbeddingReply::Snowflake(Box::new(reply)))
}

// The oci arm turns absent headers into an empty map, resolves the credentials at call time, then signs
async fn oci(
    call: &EmbeddingCall,
    context: &EmbeddingContext<'_>,
) -> Result<EmbeddingReply, DispatchError> {
    let credentials: OciCredentials = resolve_oci_credentials(
        &oci_explicit_params(&call.optional_params)?,
        &oci_environment(context.secrets, context.environment)?,
    );
    let given = call.headers.clone().unwrap_or_default();
    let mut headers = header_map(&given).map_err(|source| DispatchError::Headers { source })?;
    crate::oci_embed::validate_environment(&mut headers, &credentials, &OciEnvironment::default())?;
    let url = crate::oci_embed::get_complete_url(&credentials, call.api_base.as_deref())?;
    let params: crate::oci_embed::OciEmbedParams = convert(&call.optional_params)?;
    let input: EmbeddingInput = convert(&call.input)?;
    let request =
        crate::oci_embed::transform_embedding_request(&call.model, &input, &params, &credentials)?;
    let signed = sign_oci_request(&mut headers, &credentials, &request, &url, context.clock)?;
    let response = context
        .transport
        .post(&WireRequest {
            method: Method::POST,
            url: &url,
            headers: &headers,
            body: &signed,
            deadline: call.timeout.unwrap_or(DEFAULT_TIMEOUT),
        })
        .await?;
    let reply = crate::oci_embed::response::decode_embed_response(response.status, &response.body)?;
    Ok(EmbeddingReply::Oci(Box::new(reply)))
}
