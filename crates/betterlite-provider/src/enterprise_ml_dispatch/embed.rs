use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_bedrock::sagemaker_runtime::embedding::EmbeddingReply as SagemakerEmbeddingReply;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::compatible_wire::header_map;
use betterlite_openai::embedding::{EmbeddingInput, EmbeddingResponse};
use betterlite_openai::watsonx_audio::error::body_text;
use betterlite_openai::watsonx_auth::WatsonxAuth;
use betterlite_openai::watsonx_auth::credentials::get_watsonx_credentials;
use betterlite_openai::watsonx_auth::error::WatsonxError;
use betterlite_openai::watsonx_auth::types::WatsonxParams;
use http::Method;
use secrecy::{ExposeSecret as _, SecretString};

use crate::data_cloud_dispatch::call::EmbeddingCall;
use crate::data_cloud_dispatch::context::EnvironmentSource;
use crate::sap_auth::token::SapClock;
use crate::sap_embedding::error::get_error_class;
use crate::sap_embedding::request::transform_embedding_request;
use crate::sap_embedding::response::transform_embedding_response;
use crate::watsonx_embed::WatsonxEmbeddingConfig;
use crate::watsonx_embed::reply::decode_watsonx_embedding_reply;

use super::call::{convert, signing_params};
use super::error::EnterpriseError;
use super::sagemaker::{self, SagemakerDeps, SagemakerEmbedding};
use super::sap::{self, SapRequest};
use super::select::EmbeddingConfig;
use super::transport::{EnterpriseTransport, SharedTransport, request};
use super::watsonx;

// the replies the three embedding arms produce, kept apart as their provider types are
pub enum EmbeddingReply {
    Sap(Box<EmbeddingResponse>),
    Watsonx(Box<EmbeddingResponse>),
    Sagemaker(Box<SagemakerEmbeddingReply>),
}

pub struct EmbeddingContext<'request> {
    pub env: EnvironmentSource<'request>,
    pub sap_service_key: Option<&'request SecretString>,
    pub transport: &'request dyn EnterpriseTransport,
    pub sagemaker: SagemakerDeps<'request>,
    pub clock: &'request dyn SapClock,
}

// LiteLLM's embedding arms for the three providers, each through its own config over the shared handler steps
pub async fn dispatch(
    call: &EmbeddingCall,
    config: EmbeddingConfig,
    context: &EmbeddingContext<'_>,
) -> Result<EmbeddingReply, EnterpriseError> {
    match config {
        EmbeddingConfig::Sap => sap(call, context).await,
        EmbeddingConfig::Watsonx => Ok(EmbeddingReply::Watsonx(Box::new(
            watsonx_embedding(call, &context.env, context.transport).await?,
        ))),
        EmbeddingConfig::Sagemaker => sagemaker(call, context),
    }
}

// the sap arm runs B5845's config over B5950's token and deployment session, then posts the orchestration body
async fn sap(
    call: &EmbeddingCall,
    context: &EmbeddingContext<'_>,
) -> Result<EmbeddingReply, EnterpriseError> {
    let prepared = sap::prepare_embedding(
        SapRequest {
            // _embedding's sap arm ignores the request key: the token creator reads litellm.sap_service_key and the env
            api_key: None,
            sap_service_key: context.sap_service_key,
            env: context.env,
        },
        context.transport,
        context.clock,
    )?;
    let input: EmbeddingInput = convert(&call.input)?;
    let body = transform_embedding_request(&call.model, &input, &call.optional_params)?;
    let bytes = serde_json::to_vec(&body).map_err(|source| EnterpriseError::Encode { source })?;
    let outbound = request(
        Method::POST,
        &prepared.url,
        &prepared.headers,
        bytes,
        call.timeout,
    )?;
    let response = context
        .transport
        .send(outbound)
        .await
        .map_err(|source| EnterpriseError::Transport { source })?;
    sap_reply(&response)
}

fn sap_reply(response: &UpstreamResponse) -> Result<EmbeddingReply, EnterpriseError> {
    if !(200..300).contains(&response.status) {
        let message = std::str::from_utf8(&response.body)
            .map_err(|source| EnterpriseError::Utf8 { source })?;
        return Err(EnterpriseError::SapOrchestration(get_error_class(
            message,
            response.status,
            &response.headers,
        )));
    }
    let text =
        std::str::from_utf8(&response.body).map_err(|source| EnterpriseError::Utf8 { source })?;
    Ok(EmbeddingReply::Sap(Box::new(transform_embedding_response(
        text,
    )?)))
}

// the sagemaker arm: no key and no base, as the AWS values travel in optional params
fn sagemaker(
    call: &EmbeddingCall,
    context: &EmbeddingContext<'_>,
) -> Result<EmbeddingReply, EnterpriseError> {
    let input: EmbeddingInput = convert(&call.input)?;
    let signing = signing_params(&call.optional_params)?;
    let prepared = SagemakerEmbedding {
        model: &call.model,
        input: &input,
        optional_params: &call.optional_params,
        signing: &signing,
    };
    Ok(EmbeddingReply::Sagemaker(Box::new(sagemaker::embedding(
        &context.sagemaker,
        &prepared,
    )?)))
}

fn epoch_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

fn source_headers(call: &EmbeddingCall) -> Result<http::HeaderMap, EnterpriseError> {
    match &call.headers {
        None => Ok(http::HeaderMap::new()),
        Some(headers) => header_map(headers).map_err(|source| EnterpriseError::Headers { source }),
    }
}

// the watsonx embedding call's credentials, token, headers, url and body, over B5970's auth
async fn watsonx_request(
    call: &EmbeddingCall,
    source: &EnvironmentSource<'_>,
    transport: &dyn EnterpriseTransport,
) -> Result<(String, http::HeaderMap, Vec<u8>), EnterpriseError> {
    let environment = watsonx::environment(source)?;
    let mut params: WatsonxParams = convert(&call.optional_params)?;
    let credentials = get_watsonx_credentials(
        &mut params,
        call.api_key.as_ref(),
        call.api_base.as_deref(),
        &environment,
    )?;
    if let Some(token) = &credentials.token {
        let stored = serde_json::to_value(token.expose_secret())
            .map_err(|source| EnterpriseError::Encode { source })?;
        params.insert(String::from("token"), stored);
    }
    let auth = WatsonxAuth::new(
        environment,
        SharedTransport { inner: transport },
        Arc::new(epoch_seconds),
    )?;
    let headers = auth
        .validate_environment(
            source_headers(call)?,
            &mut params,
            Some(&credentials.api_key),
        )
        .await?;
    let config = WatsonxEmbeddingConfig {
        environment: &auth.environment,
    };
    let url = config.get_complete_url(Some(credentials.api_base.as_str()), &mut params)?;
    let input: EmbeddingInput = convert(&call.input)?;
    let body = config.transform_embedding_request(&call.model, &input, params)?;
    let bytes = serde_json::to_vec(&body).map_err(|source| EnterpriseError::Encode { source })?;
    Ok((url, headers, bytes))
}

// the watsonx embedding arm: B5970's credentials, then B5842's config over the shared handler's steps
async fn watsonx_embedding(
    call: &EmbeddingCall,
    source: &EnvironmentSource<'_>,
    transport: &dyn EnterpriseTransport,
) -> Result<EmbeddingResponse, EnterpriseError> {
    let (url, headers, body) = watsonx_request(call, source, transport).await?;
    let outbound = request(Method::POST, &url, &headers, body, call.timeout)?;
    let response = transport
        .send(outbound)
        .await
        .map_err(|source| EnterpriseError::Transport { source })?;
    if !(200..300).contains(&response.status) {
        return Err(EnterpriseError::Watsonx(WatsonxError::Api {
            status_code: response.status,
            message: body_text(&response.body),
            headers: Some(response.headers),
        }));
    }
    let reply = decode_watsonx_embedding_reply(&response.body)?;
    Ok(WatsonxEmbeddingConfig::transform_embedding_response(
        &call.model,
        reply,
    ))
}
