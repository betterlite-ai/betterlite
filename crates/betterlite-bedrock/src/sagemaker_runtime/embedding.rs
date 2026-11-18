use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json as Plain;
use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::cohere_legacy_embedding::decode::LegacyEmbeddingResponse;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::sagemaker_completion::error::SagemakerError;
use betterlite_openai::voyage_embedding::project_response;
use betterlite_prompts::arguments::json::{Json, Object as Body};
use serde::Serialize;

use crate::sagemaker_embedding::hf::{self, HfEmbeddingResponse};
use crate::sagemaker_embedding::{self, EmbeddingConfig, cohere};
use crate::signing::json_post::{JsonPost, sign_aws_json_post};

use super::convert;
use super::endpoint::{self, EndpointRequest, ParamMap};
use super::error::{RuntimeError, RuntimeFailure};
use super::runtime::{
    PreparedCall, SagemakerCredentials, SagemakerRuntime, SagemakerTransport, wire_error,
};
use super::{CONTENT_TYPE, JSON_CONTENT_TYPE};

const SERVICE: &str = "sagemaker";
const CUSTOM_ATTRIBUTES: &str = "X-Amzn-SageMaker-Custom-Attributes";
const ACCEPT_EULA: &str = "accept_eula=true";
const MOCK_STATUS: u16 = 200;

pub struct EmbeddingCall<'call> {
    pub model: &'call str,
    pub input: &'call EmbeddingInput,
    pub optional_params: &'call Body,
    pub signing: &'call ParamMap,
    pub encoding: &'call dyn TextEncoding,
}

pub struct PreparedEmbedding {
    pub call: PreparedCall,
    pub body: Body,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum EmbeddingReply {
    Voyage(Plain),
    HuggingFace(HfEmbeddingResponse),
    Cohere(LegacyEmbeddingResponse),
}

impl<T, C> SagemakerRuntime<'_, T, C>
where
    C: SagemakerCredentials,
{
    pub async fn prepare_embedding(
        &self,
        call: &EmbeddingCall<'_>,
    ) -> Result<PreparedEmbedding, RuntimeError> {
        let region = endpoint::call_region(call.signing, self.environment.inputs())?;
        let credentials = self.credentials_for(call.signing, &region).await?;
        let body = embedding_body(call.model, call.input, call.optional_params)?;
        let body_json = convert::prompt_body(&body);
        let text = dumps_object(&body_json);
        let url = endpoint::runtime_url(&EndpointRequest {
            model: call.model,
            region: &region,
            base_url: None,
            stream: false,
        });
        let signed = sign_aws_json_post(JsonPost {
            service_name: SERVICE,
            region: Some(&region),
            url: &url,
            body: &text,
            headers: &[
                (CONTENT_TYPE.to_owned(), JSON_CONTENT_TYPE.to_owned()),
                (CUSTOM_ATTRIBUTES.to_owned(), ACCEPT_EULA.to_owned()),
            ],
            credentials: &credentials,
            now: self.clock.now(),
        })?;
        Ok(PreparedEmbedding {
            call: PreparedCall {
                url: signed.url,
                headers: signed.headers,
                body: signed.body,
                body_json,
                region,
            },
            body,
        })
    }
}

impl<T, C> SagemakerRuntime<'_, T, C>
where
    T: SagemakerTransport,
    C: SagemakerCredentials,
{
    pub async fn embedding(
        &self,
        call: &EmbeddingCall<'_>,
    ) -> Result<EmbeddingReply, RuntimeFailure> {
        let prepared = self.prepare_embedding(call).await?;
        let response = self
            .transport
            .send(self.outbound(&prepared.call)?)
            .await
            .map_err(RuntimeError::from)?;
        if !(200..300).contains(&response.status) {
            return Err(RuntimeFailure::from(wire_error(
                response.status,
                &response.body,
            )));
        }
        let reply = serde_json::from_slice(&response.body)?;
        match sagemaker_embedding::get_model_config(call.model) {
            EmbeddingConfig::Voyage => Ok(EmbeddingReply::Voyage(serde_json::from_value(
                project_response(&reply),
            )?)),
            EmbeddingConfig::HuggingFace => Ok(EmbeddingReply::HuggingFace(
                hf::transform_response(&response.body, MOCK_STATUS, call.model, &prepared.body)
                    .map_err(SagemakerError::from)?,
            )),
            EmbeddingConfig::Cohere => Ok(EmbeddingReply::Cohere(
                cohere::transform_response(
                    &response.body,
                    call.model,
                    Some(call.input),
                    &prepared.body,
                    call.encoding,
                )
                .map_err(|source| SagemakerError::from(RuntimeError::from(source)))?,
            )),
        }
    }
}

fn embedding_body(
    model: &str,
    input: &EmbeddingInput,
    optional_params: &Body,
) -> Result<Body, RuntimeError> {
    match sagemaker_embedding::get_model_config(model) {
        EmbeddingConfig::Voyage => Ok(voyage_body(model, input, optional_params)),
        EmbeddingConfig::Cohere => Ok(cohere::transform_request(model, input, optional_params)?),
        EmbeddingConfig::HuggingFace => Ok(hf::transform_request(input, optional_params)),
    }
}

// VoyageEmbeddingConfig.transform_embedding_request: the input and model first, then the caller's options
fn voyage_body(model: &str, input: &EmbeddingInput, optional_params: &Body) -> Body {
    let rendered = hf::transform_request(input, &Body::new());
    let mut body = Body::new();
    if let Some(value) = rendered.get("inputs") {
        body.insert(String::from("input"), value.clone());
    }
    body.insert(String::from("model"), Json::Text(model.to_owned()));
    body.extend(optional_params.clone());
    body
}

mod tests;
