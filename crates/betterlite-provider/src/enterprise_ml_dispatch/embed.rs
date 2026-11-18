use betterlite_bedrock::sagemaker_runtime::embedding::EmbeddingReply as SagemakerEmbeddingReply;
use betterlite_openai::embedding::{EmbeddingInput, EmbeddingResponse};
use http::Method;
use secrecy::SecretString;

use crate::data_cloud_dispatch::call::EmbeddingCall;
use crate::data_cloud_dispatch::context::EnvironmentSource;
use crate::sap_auth::token::SapClock;
use crate::sap_embedding::error::get_error_class;
use crate::sap_embedding::request::transform_embedding_request;
use crate::sap_embedding::response::transform_embedding_response;

use super::call::{convert, signing_params};
use super::error::EnterpriseError;
use super::sagemaker::{self, SagemakerDeps, SagemakerEmbedding};
use super::sap::{self, SapRequest};
use super::select::EmbeddingConfig;
use super::transport::{EnterpriseTransport, request};
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
            watsonx::embedding(call, &context.env, context.transport).await?,
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
    if !(200..300).contains(&response.status) {
        let message = String::from_utf8_lossy(&response.body);
        return Err(get_error_class(&message, response.status, &response.headers).into());
    }
    let text = String::from_utf8_lossy(&response.body);
    Ok(EmbeddingReply::Sap(Box::new(transform_embedding_response(
        &text,
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
