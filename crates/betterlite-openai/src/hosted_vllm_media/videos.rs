use betterlite_config::loader::EnvironmentLookup;
use betterlite_http::user_url::error::{SsrfError, UrlParseError};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use thiserror::Error;

use crate::openai_base_config::Headers;
use crate::openai_video_identifiers::multipart::{MediaBody, add_image_to_files};
use crate::videos::VideoOptions;
use crate::videos::config::{SUPPORTED_PARAMS, map_params};
use crate::videos::create::{CreateVideoForm, VideoCreate};

use super::{API_KEY_ENV, PLACEHOLDER_KEY, resolved_api_base, versioned_url};

mod form;
mod tests;

// _VLLM_OMNI_VIDEO_PARAMS: the extra form fields vLLM-Omni accepts on top of the OpenAI ones
pub const VLLM_OMNI_VIDEO_PARAMS: [&str; 24] = [
    "image_reference",
    "video_reference",
    "audio_reference",
    "width",
    "height",
    "num_frames",
    "fps",
    "num_inference_steps",
    "guidance_scale",
    "guidance_scale_2",
    "boundary_ratio",
    "flow_shift",
    "true_cfg_scale",
    "seed",
    "generate_sound",
    "sound_duration",
    "negative_prompt",
    "enable_frame_interpolation",
    "frame_interpolation_exp",
    "frame_interpolation_scale",
    "frame_interpolation_model_path",
    "lora",
    "extra_params",
    "aspect_ratio",
];

// _EXCLUDED_FORM_KEYS: the values that travel outside the form fields
const EXCLUDED_FORM_KEYS: [&str; 9] = [
    "model",
    "prompt",
    "extra_headers",
    "extra_query",
    "extra_body",
    "timeout",
    "custom_llm_provider",
    "input_reference",
    "characters",
];

#[derive(Debug, Error)]
pub enum HostedVllmVideoError {
    #[error(
        "api_base not set for Hosted VLLM videos API. Set via api_base parameter or HOSTED_VLLM_API_BASE environment variable"
    )]
    MissingApiBase,
    #[error(transparent)]
    UnsafeUrl(#[from] SsrfError),
    #[error("the media URL cannot be parsed")]
    UrlParse {
        #[source]
        source: UrlParseError,
    },
    #[error("a media reference is not valid JSON")]
    MediaJson {
        #[source]
        source: serde_json::Error,
    },
}

// get_supported_openai_params is the OpenAI list followed by the vLLM-Omni fields, in LiteLLM's order
#[must_use]
pub fn get_supported_openai_params() -> Vec<&'static str> {
    SUPPORTED_PARAMS
        .iter()
        .copied()
        .chain(VLLM_OMNI_VIDEO_PARAMS)
        .collect()
}

// map_openai_params copies every option the caller set, supported or not, and drops only the nulls
#[must_use]
pub fn map_openai_params(options: &VideoCreate) -> VideoOptions {
    let mut mapped = VideoOptions::new();
    map_params(options, &mut mapped);
    mapped.retain(|_name, value| !value.is_null());
    mapped
}

// validate_environment prefers the call's own key over the model's, then the environment, then the placeholder
#[must_use]
pub fn validate_environment(
    mut headers: Headers,
    api_key: Option<&SecretString>,
    params_api_key: Option<&SecretString>,
    environment: &dyn EnvironmentLookup,
) -> Headers {
    let resolved = [params_api_key, api_key]
        .into_iter()
        .flatten()
        .map(|key| key.expose_secret())
        .find(|key| !key.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            environment
                .lookup(API_KEY_ENV)
                .filter(|key| !key.is_empty())
        })
        .unwrap_or_else(|| String::from(PLACEHOLDER_KEY));
    headers.insert(String::from("Authorization"), format!("Bearer {resolved}"));
    headers
}

pub fn get_complete_url(
    api_base: Option<&str>,
    environment: &dyn EnvironmentLookup,
) -> Result<String, HostedVllmVideoError> {
    let base =
        resolved_api_base(api_base, environment).ok_or(HostedVllmVideoError::MissingApiBase)?;
    Ok(versioned_url(&base, "/videos"))
}

// transform_video_create_request writes model and prompt, then every remaining option as a form string
pub fn transform_video_create_request<'media>(
    model: &str,
    prompt: &str,
    api_base: &str,
    params: &VideoOptions,
    input_reference: Option<MediaBody<'media>>,
) -> Result<CreateVideoForm<'media>, HostedVllmVideoError> {
    let mut fields = VideoOptions::new();
    fields.insert(String::from("model"), Value::String(model.to_owned()));
    fields.insert(String::from("prompt"), Value::String(prompt.to_owned()));
    for (name, value) in params {
        if EXCLUDED_FORM_KEYS.contains(&name.as_str()) || value.is_null() {
            continue;
        }
        let rendered = form::form_value(name, value)?;
        fields.insert(name.clone(), Value::String(rendered));
    }
    let mut files = Vec::new();
    if let Some(reference) = input_reference {
        add_image_to_files(&mut files, "input_reference", reference);
    }
    Ok(CreateVideoForm {
        fields,
        files,
        api_base: api_base.to_owned(),
    })
}
