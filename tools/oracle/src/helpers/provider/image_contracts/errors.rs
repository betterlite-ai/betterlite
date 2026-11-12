use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;

use betterlite_openai::api::image_contracts::records::{
    ImageFile, ImageHeaders, ImageHttpResponse, ImageParams, ImageResponse,
};
use betterlite_openai::error::ProviderError;
use betterlite_provider::api::image_contracts::edit::BaseImageEditConfig;
use betterlite_provider::api::image_contracts::generation::{
    BaseImageGenerationConfig, ImageGenerationReply, ImageGenerationRequest,
};
use betterlite_provider::api::image_contracts::variation::{
    BaseImageVariationConfig, ImageVariationChat, ImageVariationReply,
};
use serde_json::{Value, json};

use super::fixtures::{EditFixture, GenerationFixture, VariationFixture};
use super::{BaseErrors, MODEL};

pub(super) fn base_errors(fields: &BaseErrors) -> Value {
    let generation = BaseImageGenerationConfig::get_error_class(
        &GenerationFixture,
        &fields.message,
        fields.status_code,
        &fields.headers,
    );
    let edit = BaseImageEditConfig::get_error_class(
        &EditFixture {
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
            raise_status: None,
        },
        &fields.message,
        fields.status_code,
        &fields.headers,
    );
    let (generation_request, variation_request) = request_probes(&fields.headers);
    let (generation_response, variation_response) = response_probes();
    json!({
        "generation": error_value(&generation),
        "edit": error_value(&edit),
        "generation_request": generation_request,
        "generation_response": generation_response,
        "variation_request": variation_request,
        "variation_response": variation_response,
    })
}

fn error_value(error: &ProviderError) -> Value {
    json!({
        "status_code": error.status_code,
        "message": error.message,
        "headers": error.headers,
    })
}

fn request_probes(headers: &ImageHeaders) -> (String, String) {
    let params = ImageParams::new();
    let generation_request = ImageGenerationRequest {
        model: MODEL,
        prompt: "a cat",
        optional_params: &params,
        litellm_params: &params,
        headers,
    };
    let variation_chat = ImageVariationChat {
        model: MODEL,
        messages: &[],
        optional_params: &params,
        litellm_params: &params,
        headers,
    };
    (
        raised(GenerationFixture.transform_image_generation_request(generation_request)),
        raised(VariationFixture.transform_request(variation_chat)),
    )
}

fn response_probes() -> (String, String) {
    let params = ImageParams::new();
    let raw_response = ImageHttpResponse::default();
    let mut generation_model_response = ImageResponse::default();
    let generation_reply = ImageGenerationReply {
        model: MODEL,
        raw_response: &raw_response,
        model_response: &mut generation_model_response,
        request_data: &params,
        optional_params: &params,
        litellm_params: &params,
        api_key: None,
        json_mode: None,
    };
    let mut variation_model_response = ImageResponse::default();
    (
        raised(GenerationFixture.transform_image_generation_response(generation_reply)),
        raised(VariationFixture.transform_response(ImageVariationReply {
            model: None,
            raw_response: &raw_response,
            model_response: &mut variation_model_response,
            request_data: &params,
            image: &ImageFile::default(),
            optional_params: &params,
            litellm_params: &params,
            api_key: None,
        })),
    )
}

fn raised<T, E>(outcome: Result<T, E>) -> String
where
    E: std::fmt::Display,
{
    match outcome {
        Ok(_) => String::from("no error"),
        Err(error) => error.to_string(),
    }
}
