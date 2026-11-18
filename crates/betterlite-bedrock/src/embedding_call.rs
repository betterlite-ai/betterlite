use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

use crate::aws_endpoints::provider::{BedrockEmbeddingProvider, embedding_provider};

const TITAN_MODELS: [&str; 4] = [
    "amazon.titan-embed-image-v1",
    "amazon.titan-embed-text-v1",
    "amazon.titan-embed-text-v2:0",
    "amazon.titan-embed-g1-text-02",
];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EmbeddingCallError {
    #[error(
        "Unable to determine bedrock embedding provider for model: {model}. Supported providers: ['cohere', 'amazon', 'twelvelabs', 'nova']"
    )]
    Provider { model: String },
    #[error("Unable to map Bedrock request to provider")]
    Unmapped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingPlan {
    pub model: String,
    pub model_id: String,
    pub endpoint_url: String,
    pub provider: BedrockEmbeddingProvider,
    pub asynchronous: bool,
    pub batch: Vec<Object>,
    pub cohere_data: Option<Object>,
    pub inference_params: Object,
}

pub struct PlanInput<'a> {
    pub model: &'a str,
    pub model_id: Option<&'a str>,
    pub optional_params: &'a Object,
    pub endpoint_url: &'a str,
    pub inputs: &'a [String],
    pub drop_params: bool,
}

pub fn plan(input: PlanInput<'_>) -> Result<EmbeddingPlan, EmbeddingCallError> {
    let unencoded_id = input
        .model_id
        .filter(|value| !value.is_empty())
        .unwrap_or(input.model);
    let model_id = encode_model_id(unencoded_id);
    let asynchronous = input.model.contains("async_invoke/");
    let model = if asynchronous {
        input.model.replacen("async_invoke/", "", 1)
    } else {
        input.model.to_owned()
    };
    let provider = embedding_provider(&model).ok_or_else(|| EmbeddingCallError::Provider {
        model: model.clone(),
    })?;
    let mut inference_params = input.optional_params.clone();
    inference_params.retain(|key, _| !is_auth_key(key) && key != "user");
    let mut batch = Vec::new();
    let mut cohere_data = None;
    match provider {
        BedrockEmbeddingProvider::Cohere => {
            cohere_data = Some(crate::titan_embeddings::cohere::transform_request(
                &model,
                input.inputs,
                &inference_params,
            ));
        }
        BedrockEmbeddingProvider::Amazon if TITAN_MODELS.contains(&model.as_str()) => {
            for text in input.inputs {
                let body = match model.as_str() {
                    "amazon.titan-embed-image-v1" => {
                        crate::titan_embeddings::multimodal::transform_request(
                            text,
                            &inference_params,
                        )
                    }
                    "amazon.titan-embed-text-v1" | "amazon.titan-embed-g1-text-02" => {
                        crate::titan_embeddings::g1::transform_request(text)
                    }
                    "amazon.titan-embed-text-v2:0" => {
                        crate::titan_embeddings::v2::transform_request(text, &inference_params)
                            .map_err(|_| EmbeddingCallError::Unmapped)?
                    }
                    _ => return Err(EmbeddingCallError::Unmapped),
                };
                batch.push(body);
            }
        }
        BedrockEmbeddingProvider::Twelvelabs | BedrockEmbeddingProvider::Nova => {
            for text in input.inputs {
                let params = params_from_object(&inference_params)?;
                let body = if provider == BedrockEmbeddingProvider::Nova {
                    let request = crate::nova_embeddings::request::transform_request(
                        text,
                        &params,
                        asynchronous,
                        Some(&model_id),
                        input
                            .optional_params
                            .get("output_s3_uri")
                            .and_then(Json::as_str),
                    )
                    .map_err(|_| EmbeddingCallError::Unmapped)?;
                    object_from_value(request)?
                } else {
                    let request = crate::marengo_embeddings::request::transform_request(
                        crate::marengo_embeddings::request::TransformRequest {
                            model: Some(&model),
                            input: text,
                            inference_params: &params,
                            async_invoke_route: asynchronous,
                            model_id: Some(&model_id),
                            output_s3_uri: input
                                .optional_params
                                .get("output_s3_uri")
                                .and_then(Json::as_str),
                            drop_params: input.drop_params,
                        },
                    )
                    .map_err(|_| EmbeddingCallError::Unmapped)?;
                    object_from_value(request)?
                };
                batch.push(body);
            }
        }
        BedrockEmbeddingProvider::Amazon => return Err(EmbeddingCallError::Unmapped),
    }
    let suffix = if asynchronous { "/async-invoke" } else { "" };
    let url = if asynchronous {
        format!("{}{suffix}", input.endpoint_url)
    } else {
        format!("{}/model/{model_id}/invoke", input.endpoint_url)
    };
    Ok(EmbeddingPlan {
        model,
        model_id,
        endpoint_url: url,
        provider,
        asynchronous,
        batch,
        cohere_data,
        inference_params,
    })
}

fn encode_model_id(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn is_auth_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "aws_access_key_id"
            | "aws_secret_access_key"
            | "aws_session_token"
            | "aws_bedrock_runtime_endpoint"
            | "aws_region_name"
            | "aws_region"
            | "aws_profile_name"
            | "aws_role_name"
            | "aws_role_arn"
            | "aws_session_name"
            | "aws_external_id"
            | "aws_web_identity_token"
            | "aws_sts_endpoint"
            | "aws_bedrock_custom_auth"
    )
}

fn params_from_object(
    value: &Object,
) -> Result<crate::image_models::params::Params, EmbeddingCallError> {
    let value = serde_json::to_value(value).map_err(|_| EmbeddingCallError::Unmapped)?;
    serde_json::from_value(value).map_err(|_| EmbeddingCallError::Unmapped)
}

fn object_from_value<T: serde::Serialize>(value: T) -> Result<Object, EmbeddingCallError> {
    let value = serde_json::to_value(value).map_err(|_| EmbeddingCallError::Unmapped)?;
    serde_json::from_value(value).map_err(|_| EmbeddingCallError::Unmapped)
}

#[cfg(test)]
mod tests;
