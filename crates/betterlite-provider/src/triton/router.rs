use serde::Serialize;

use super::config::TritonEndpoint;
use super::error::TritonError;
use super::generate::{GenerateReply, GenerateRequest, TritonGenerateConfig};
use super::infer::{InferReply, InferRequest};
use super::request::TransformInput;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum TritonRequest {
    Generate(GenerateRequest),
    Infer(InferRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum TritonReply {
    Generate(GenerateReply),
    Infer(InferReply),
}

pub async fn transform_request(
    config: &TritonGenerateConfig<'_>,
    api_base: &str,
    input: &TransformInput<'_>,
) -> Result<TritonRequest, TritonError> {
    match TritonEndpoint::from_api_base(api_base)? {
        TritonEndpoint::Generate => config
            .transform_request(input)
            .await
            .map(TritonRequest::Generate),
        TritonEndpoint::Infer => super::infer::transform_request(input).map(TritonRequest::Infer),
    }
}

pub fn transform_response(
    api_base: &str,
    status_code: u16,
    body: &[u8],
) -> Result<TritonReply, TritonError> {
    match TritonEndpoint::from_api_base(api_base)? {
        TritonEndpoint::Generate => {
            super::generate::transform_response(status_code, body).map(TritonReply::Generate)
        }
        TritonEndpoint::Infer => {
            super::infer::transform_response(status_code, body).map(TritonReply::Infer)
        }
    }
}
