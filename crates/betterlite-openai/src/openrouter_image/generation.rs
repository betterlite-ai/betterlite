use super::decode::decode_openrouter_image_reply;
use super::shared::{
    CHAT_COMPLETIONS, DEFAULT_API_BASE, ImageConfigError, ImageHiddenParams, SKIPPED_PARAMS,
    SUPPORTED_PARAMS, apply_generation_usage, authorization, map_quality_to_image_size,
    map_size_to_aspect_ratio, reply_error, resolved_key, transform_error,
};
use crate::api::image_contracts::records::{ImageHttpResponse, ImageParams, ImageResponse};
use crate::openai_base_config::Headers;
use crate::openrouter_chat::error::OpenRouterError;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
// ast-grep-ignore: no-long-names
use self::OpenRouterImageGenerationConfig as GenerationConfig;
const STAGE: &str = "image generation";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// ast-grep-ignore: no-long-names
pub struct OpenRouterImageGenerationConfig {
    pub supported_params: [&'static str; 3],
}
#[must_use]
// ast-grep-ignore: no-long-names
pub const fn get_openrouter_image_generation_config(_model: &str) -> GenerationConfig {
    GenerationConfig {
        supported_params: SUPPORTED_PARAMS,
    }
}
impl GenerationConfig {
    pub fn map_params(
        &self,
        non_default_params: &ImageParams,
        mut optional_params: ImageParams,
        drop_params: bool,
    ) -> Result<ImageParams, ImageConfigError> {
        for (name, value) in non_default_params {
            if !self.supported_params.contains(&name.as_str()) {
                if !drop_params {
                    optional_params.insert(name.clone(), value.clone());
                }
                continue;
            }
            match name.as_str() {
                "size" => {
                    let Json::Text(size) = value else {
                        return Err(ImageConfigError::Size);
                    };
                    image_config(&mut optional_params)?.insert(
                        String::from("aspect_ratio"),
                        Json::Text(map_size_to_aspect_ratio(size).to_owned()),
                    );
                }
                "quality" => {
                    let Json::Text(quality) = value else {
                        return Err(ImageConfigError::Quality);
                    };
                    if let Some(image_size) = map_quality_to_image_size(quality) {
                        image_config(&mut optional_params)?.insert(
                            String::from("image_size"),
                            Json::Text(image_size.to_owned()),
                        );
                    }
                }
                _ => {
                    optional_params.insert(name.clone(), value.clone());
                }
            }
        }
        Ok(optional_params)
    }
}
fn image_config(
    optional_params: &mut ImageParams,
) -> Result<&mut IndexMap<String, Json>, ImageConfigError> {
    match optional_params
        .entry(String::from("image_config"))
        .or_insert_with(|| Json::Object(IndexMap::new()))
    {
        Json::Object(config) => Ok(config),
        _ => Err(ImageConfigError::ImageConfig),
    }
}
#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    if let Some(base) = api_base
        && !base.is_empty()
    {
        if base.ends_with(CHAT_COMPLETIONS) {
            return base.to_owned();
        }
        let trimmed = base.trim_end_matches('/');
        return format!("{trimmed}{CHAT_COMPLETIONS}");
    }
    format!("{DEFAULT_API_BASE}{CHAT_COMPLETIONS}")
}
#[must_use]
pub fn validate_environment(
    mut headers: Headers,
    api_key: Option<&SecretString>,
    global_api_key: Option<&SecretString>,
    environment_key: Option<&SecretString>,
) -> Headers {
    let key = resolved_key([api_key, global_api_key, environment_key]);
    headers.insert(String::from("Authorization"), authorization(key));
    headers
}
#[must_use]
pub fn request_body(model: &str, prompt: &str, optional_params: &ImageParams) -> ImageParams {
    let mut body = ImageParams::new();
    body.insert(String::from("model"), Json::Text(model.to_owned()));
    body.insert(
        String::from("messages"),
        Json::List(vec![user_message(prompt)]),
    );
    for (name, value) in optional_params {
        if !SKIPPED_PARAMS.contains(&name.as_str()) {
            body.insert(name.clone(), value.clone());
        }
    }
    body
}
fn user_message(prompt: &str) -> Json {
    Json::Object(IndexMap::from([
        (String::from("role"), Json::Text(String::from("user"))),
        (String::from("content"), Json::Text(prompt.to_owned())),
    ]))
}
pub fn transform_response(
    raw: &ImageHttpResponse,
    mut model_response: ImageResponse,
    mut hidden: ImageHiddenParams,
    model: &str,
) -> Result<(ImageResponse, ImageHiddenParams), OpenRouterError> {
    let reply =
        decode_openrouter_image_reply(&raw.body).map_err(|error| reply_error(error, raw, STAGE))?;
    let mut data = model_response.data.take().unwrap_or_default();
    data.extend(reply.images);
    model_response.data = Some(data);
    if let Some(usage) = &reply.usage {
        apply_generation_usage(usage, &mut model_response, &mut hidden)
            .map_err(|error| transform_error(STAGE, &error.to_string()))?;
    }
    hidden.model = Some(reply.model.unwrap_or_else(|| Json::Text(model.to_owned())));
    Ok((model_response, hidden))
}
