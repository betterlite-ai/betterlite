use betterlite_context::media_optional_params::image_generation::{
    ImageGenerationRequest, get_optional_params_image_gen, vertex_aspect_ratio,
};
use betterlite_context::media_optional_params::transcription::{
    TranscriptionRequest, transcription_optional_params,
};
use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::mapper::{DropPolicy, OptionalParamsMapper};
use serde::Deserialize;
use serde_json::Value;

use super::Failure;
use super::fixture::MapperSpec;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TranscriptionInput {
    model: String,
    custom_llm_provider: String,
    params: Params,
    drop_params: Option<bool>,
    #[serde(default)]
    litellm_drop_params: bool,
    mapper: Option<MapperSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ImageInput {
    model: Option<String>,
    custom_llm_provider: Option<String>,
    params: Params,
    additional_drop_params: Option<Vec<String>>,
    drop_params: Option<bool>,
    #[serde(default)]
    litellm_drop_params: bool,
    provider_config: Option<MapperSpec>,
    bedrock: Option<MapperSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum MediaCall {
    Transcription(TranscriptionInput),
    ImageGeneration(ImageInput),
    VertexAspectRatio { size: Option<String> },
}

fn transcription(input: &TranscriptionInput) -> Result<Value, Failure> {
    let request = TranscriptionRequest {
        model: &input.model,
        custom_llm_provider: &input.custom_llm_provider,
        params: &input.params,
        drop_policy: DropPolicy {
            drop_params: input.drop_params,
            litellm_drop_params: input.litellm_drop_params,
        },
        mapper: input
            .mapper
            .as_ref()
            .map(|fixture| -> &dyn OptionalParamsMapper { fixture }),
    };
    match transcription_optional_params(&request) {
        Ok(optional) => Ok(serde_json::to_value(optional)?),
        Err(error) => Err(Failure::Status(error.status_code())),
    }
}

fn image_generation(input: &ImageInput) -> Result<Value, Failure> {
    let request = ImageGenerationRequest {
        model: input.model.as_deref(),
        custom_llm_provider: input.custom_llm_provider.as_deref(),
        params: &input.params,
        additional_drop_params: input.additional_drop_params.as_deref().unwrap_or_default(),
        drop_policy: DropPolicy {
            drop_params: input.drop_params,
            litellm_drop_params: input.litellm_drop_params,
        },
        provider_config: input
            .provider_config
            .as_ref()
            .map(|fixture| -> &dyn OptionalParamsMapper { fixture }),
        bedrock_config: input
            .bedrock
            .as_ref()
            .map(|fixture| -> &dyn OptionalParamsMapper { fixture }),
    };
    match get_optional_params_image_gen(&request) {
        Ok(optional) => Ok(serde_json::to_value(optional)?),
        Err(error) => Err(Failure::Status(error.status_code())),
    }
}

pub(super) fn run(call: &MediaCall) -> Result<Value, Failure> {
    match call {
        MediaCall::Transcription(input) => transcription(input),
        MediaCall::ImageGeneration(input) => image_generation(input),
        MediaCall::VertexAspectRatio { size } => Ok(Value::String(
            vertex_aspect_ratio(size.as_deref()).to_owned(),
        )),
    }
}
