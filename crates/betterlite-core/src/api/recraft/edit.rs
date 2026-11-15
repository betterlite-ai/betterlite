use std::io::Cursor;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{
    ImageFile, ImageFiles, ImageHeaders, ImageHttpResponse, ImageParams, ImageResponse,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use super::error::RecraftError;
use super::reply::{RecraftReplyError, decode_recraft_images};
use super::settings::RecraftImageSettings;
use super::types::RecraftImageEditRequest;
use crate::api::image_input_options::mime::image_content_type_seekable;

pub const DEFAULT_STRENGTH: f64 = 0.2;
pub const DEFAULT_IMAGE_NAME: &str = "image.png";
const IMAGE_FIELD: &str = "image";

#[derive(Debug, Clone)]
pub struct RecraftImageEditConfig {
    pub settings: RecraftImageSettings,
}

impl RecraftImageEditConfig {
    // LiteLLM's get_supported_openai_params answers these three names for every model
    pub const SUPPORTED_OPENAI_PARAMS: [&'static str; 3] = ["n", "response_format", "style"];

    pub const ENDPOINT: &'static str = "v1/images/imageToImage";

    #[must_use]
    pub fn get_complete_url(&self, api_base: Option<&str>) -> String {
        let base = api_base
            .filter(|base| !base.is_empty())
            .unwrap_or(self.settings.api_base.as_str());
        format!("{}/{}", base.trim_end_matches('/'), Self::ENDPOINT)
    }

    pub fn validate_environment(
        &self,
        mut headers: ImageHeaders,
        api_key: Option<&SecretString>,
    ) -> Result<ImageHeaders, RecraftError> {
        let key = api_key
            .filter(|key| !key.expose_secret().is_empty())
            .or(self.settings.api_key.as_ref())
            .ok_or(RecraftError::MissingApiKey)?;
        headers.insert(
            "Authorization".to_owned(),
            format!("Bearer {}", key.expose_secret()),
        );
        Ok(headers)
    }

    // LiteLLM filters to the names above whatever drop_params says, unlike the generation mapping
    #[must_use]
    pub fn map_openai_params(
        image_edit_optional_params: ImageParams,
        _model: &str,
        _drop_params: bool,
    ) -> ImageParams {
        image_edit_optional_params
            .into_iter()
            .filter(|(name, _)| Self::SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()))
            .collect()
    }

    #[must_use]
    pub fn transform_image_edit_request(
        model: &str,
        prompt: Option<&str>,
        image: Option<&[ImageFile]>,
        optional_params: ImageParams,
    ) -> (RecraftImageEditRequest, ImageFiles) {
        let mut options = optional_params;
        let body_model = options
            .shift_remove("model")
            .unwrap_or_else(|| Json::Text(model.to_owned()));
        let strength = options
            .shift_remove("strength")
            .filter(|value| !matches!(value, Json::Null))
            .unwrap_or_else(|| Number::from_f64(DEFAULT_STRENGTH).map_or(Json::Null, Json::Number));
        options.shift_remove(IMAGE_FIELD);
        let supplied_prompt = options
            .shift_remove("prompt")
            .filter(|value| !matches!(value, Json::Null));
        let body_prompt = prompt.map_or(supplied_prompt, |text| Some(Json::Text(text.to_owned())));
        let files = image.and_then(|images| images.first()).map(image_file);
        let body = RecraftImageEditRequest {
            model: body_model,
            strength,
            prompt: body_prompt,
            options,
        };
        (body, files.into_iter().collect())
    }

    #[must_use]
    pub fn transform_image_edit_response(
        response: &ImageHttpResponse,
        created: i64,
    ) -> Result<ImageResponse, RecraftReplyError> {
        let images = decode_recraft_images(response)?;
        Ok(ImageResponse {
            created,
            data: Some(images),
            usage: None,
        })
    }
}

fn image_file(image: &ImageFile) -> (String, ImageFile) {
    let mut reader = Cursor::new(image.content.as_slice());
    let content_type = image_content_type_seekable(&mut reader);
    (
        IMAGE_FIELD.to_owned(),
        ImageFile {
            filename: Some(
                image
                    .filename
                    .clone()
                    .unwrap_or_else(|| DEFAULT_IMAGE_NAME.to_owned()),
            ),
            content: image.content.clone(),
            content_type: Some(content_type.to_owned()),
        },
    )
}
