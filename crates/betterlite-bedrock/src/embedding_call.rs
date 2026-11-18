use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;
use std::time::SystemTime;
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
    pub optional_params: &'a mut Object,
    pub endpoint_url: &'a str,
    pub inputs: &'a [String],
    pub drop_params: bool,
}

pub fn plan(input: PlanInput<'_>) -> Result<EmbeddingPlan, EmbeddingCallError> {
    let explicit_model_id = input
        .optional_params
        .shift_remove("model_id")
        .and_then(|value| value.as_str().map(str::to_owned));
    let unencoded_id = explicit_model_id
        .as_deref()
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
    let _runtime_endpoint = input
        .optional_params
        .shift_remove("aws_bedrock_runtime_endpoint");
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

pub async fn execute_batch<Factory>(
    plan: &EmbeddingPlan,
    timeout: Option<crate::bedrock_embedding_io::transport::Timeout>,
    headers: &[(String, String)],
    supplied: Option<(
        crate::bedrock_embedding_io::transport::ClientKind,
        &Factory::Client,
    )>,
    factory: &Factory,
) -> Result<
    crate::bedrock_embedding_io::response::EmbeddingReply,
    crate::bedrock_embedding_io::error::EmbeddingIoError,
>
where
    Factory: crate::bedrock_embedding_io::transport::ClientFactory + Sync,
{
    let batch_data = if let Some(data) = &plan.cohere_data {
        std::slice::from_ref(data)
    } else {
        plan.batch.as_slice()
    };
    let mut responses = Vec::with_capacity(batch_data.len());
    for data in batch_data {
        let supplied = supplied.map(|(kind, transport)| {
            crate::bedrock_embedding_io::transport::SuppliedClient { kind, transport }
        });
        let response = crate::bedrock_embedding_io::transport::post_embedding(
            &crate::bedrock_embedding_io::transport::EmbeddingCall {
                asynchronous: plan.asynchronous,
                url: &plan.endpoint_url,
                headers,
                data,
                timeout: timeout.clone(),
            },
            supplied,
            factory,
        )
        .await?;
        responses.push(response);
    }
    crate::bedrock_embedding_io::response::transform_response(
        &responses,
        &plan.model,
        plan.provider,
        plan.asynchronous,
        Some(batch_data),
    )
}

pub struct SigningInput<'a> {
    pub plan: &'a EmbeddingPlan,
    pub data: &'a Object,
    pub credentials: Option<&'a crate::aws_settings::sources::Credentials>,
    pub region: &'a str,
    pub extra_headers: &'a [(String, String)],
    pub api_key: Option<&'a SecretString>,
    pub bearer_token: Option<&'a SecretString>,
    pub now: SystemTime,
}

pub fn sign_request(
    input: SigningInput<'_>,
) -> Result<crate::signing::request::AwsSignedRequest, crate::signing::error::SigningError> {
    let body = betterlite_logging::python_json::dumps_object(input.data).into_bytes();
    let headers = merged_headers(input.extra_headers);
    crate::signing::endpoint::request_headers(crate::signing::endpoint::RequestHeaders {
        credentials: input.credentials,
        region: input.region,
        extra_headers: input.extra_headers,
        endpoint: &input.plan.endpoint_url,
        data: crate::signing::body::Body::Bytes(&body),
        headers: &headers,
        api_key: input.api_key,
        configured_bearer: input.bearer_token,
        supports_bearer_token: true,
        now: input.now,
    })
}

pub struct AsyncSigningInput {
    pub plan: EmbeddingPlan,
    pub data: Object,
    pub credentials: Option<crate::aws_settings::sources::Credentials>,
    pub region: String,
    pub extra_headers: Vec<(String, String)>,
    pub api_key: Option<SecretString>,
    pub bearer_token: Option<SecretString>,
    pub now: SystemTime,
}

pub struct LogContext<'state, 'env> {
    pub state: &'state mut betterlite_logging::logging_state::LoggingState,
    pub env: &'state betterlite_logging::logging_pre_post::env::PrePostEnv<'env>,
}

pub fn log_pre_call(
    data: &Object,
    signed: &crate::signing::request::AwsSignedRequest,
    log: &mut LogContext<'_, '_>,
) -> Result<(), betterlite_logging::logging_pre_post::error::PrePostError> {
    let mut headers = indexmap::IndexMap::new();
    for (name, value) in &signed.headers {
        headers.insert(
            name.clone(),
            betterlite_models::decisions::Json::Text(value.clone()),
        );
    }
    let value = model_json(data);
    betterlite_logging::logging_pre_post::pre_call::pre_call(
        log.state,
        &betterlite_logging::logging_pre_post::pre_call::PreCall {
            input: betterlite_models::decisions::Json::Object(value.clone()),
            model: None,
            additional_args: betterlite_logging::logging_pre_post::args::AdditionalArgs {
                api_base: Some(signed.url.clone()),
                headers: Some(headers),
                complete_input_dict: Some(
                    betterlite_logging::logging_pre_post::args::RawBody::Object(value),
                ),
                request_str: None,
                openai_sdk: None,
                passthrough: indexmap::IndexMap::new(),
            },
        },
        log.env,
    )?;
    Ok(())
}

pub fn log_post_call(data: &Object, response: &Object, log: &mut LogContext<'_, '_>) {
    let input = model_json(data);
    let response = model_json(response);
    betterlite_logging::logging_pre_post::post_call::post_call(
        log.state,
        &betterlite_logging::logging_pre_post::post_call::PostCall {
            original_response: betterlite_models::decisions::Json::Object(response),
            input: betterlite_models::decisions::Json::Object(input.clone()),
            additional_args: betterlite_logging::logging_pre_post::args::AdditionalArgs {
                api_base: None,
                headers: None,
                complete_input_dict: Some(
                    betterlite_logging::logging_pre_post::args::RawBody::Object(input),
                ),
                request_str: None,
                openai_sdk: None,
                passthrough: indexmap::IndexMap::new(),
            },
        },
        log.env,
    );
}

fn model_json(value: &Object) -> indexmap::IndexMap<String, betterlite_models::decisions::Json> {
    value
        .iter()
        .map(|(key, field)| (key.clone(), model_json_value(field)))
        .collect()
}

fn model_json_value(value: &Json) -> betterlite_models::decisions::Json {
    use betterlite_models::decisions::Json as Output;
    match value {
        Json::Null => Output::Null,
        Json::Bool(value) => Output::Bool(*value),
        Json::Number(value) => Output::Number(value.clone()),
        Json::Text(value) => Output::Text(value.clone()),
        Json::List(values) => Output::List(values.iter().map(model_json_value).collect()),
        Json::Object(values) => Output::Object(model_json(values)),
    }
}

pub async fn sign_request_async(
    executor: &crate::signing::executor::AwsSigningExecutor,
    input: AsyncSigningInput,
) -> Result<crate::signing::request::AwsSignedRequest, crate::signing::error::SigningError> {
    let url = input.plan.endpoint_url;
    let has_bearer = crate::aws_settings::bedrock_bearer_token(
        input.api_key.as_ref(),
        input.bearer_token.as_ref(),
    )
    .is_some();
    crate::signing::executor::sign_request_off_loop_if_aws(executor, !has_bearer, move || {
        let body = betterlite_logging::python_json::dumps_object(&input.data).into_bytes();
        let headers = merged_headers(&input.extra_headers);
        crate::signing::endpoint::request_headers(crate::signing::endpoint::RequestHeaders {
            credentials: input.credentials.as_ref(),
            region: &input.region,
            extra_headers: &input.extra_headers,
            endpoint: &url,
            data: crate::signing::body::Body::Bytes(&body),
            headers: &headers,
            api_key: input.api_key.as_ref(),
            configured_bearer: input.bearer_token.as_ref(),
            supports_bearer_token: true,
            now: input.now,
        })
    })
    .await?
}

fn merged_headers(extra_headers: &[(String, String)]) -> Vec<(String, Option<String>)> {
    let mut headers = vec![(
        "Content-Type".to_owned(),
        Some("application/json".to_owned()),
    )];
    for (name, value) in extra_headers {
        match headers.iter_mut().find(|(existing, _)| existing == name) {
            Some((_, current)) => *current = Some(value.clone()),
            None => headers.push((name.clone(), Some(value.clone()))),
        }
    }
    headers
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
    crate::aws_settings::AUTHENTICATION_PARAMS
        .iter()
        .any(|name| key.eq_ignore_ascii_case(name))
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
