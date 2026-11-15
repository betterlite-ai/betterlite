use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageFiles, ImageParams};
use betterlite_openai::image_edits::request::ImageEditForm;
use betterlite_openai::image_generation::reply::ImageDatum;

use self::media::{MaskInput, SuppliedMedia, is_remote_candidate, read_media_bytes};
use crate::api::bfl::common::{BflError, BflResult, IMAGE_EDIT_MODELS, decode_bfl_reply};
use crate::api::bfl::generation::BflImageResponse;

pub mod media;
mod tests;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 13] = [
    "mask",
    "seed",
    "output_format",
    "safety_tolerance",
    "prompt_upsampling",
    "aspect_ratio",
    "steps",
    "guidance",
    "grow_mask",
    "top",
    "bottom",
    "left",
    "right",
];

const REQUEST_PARAMS: [&str; 12] = [
    "seed",
    "output_format",
    "safety_tolerance",
    "prompt_upsampling",
    "aspect_ratio",
    "steps",
    "guidance",
    "grow_mask",
    "top",
    "bottom",
    "left",
    "right",
];

const DEFAULT_OUTPUT_FORMAT: &str = "png";
// BlackForestLabsImageEditConfig.use_multipart_form_data
pub const MULTIPART_FORM_DATA: bool = false;
const IMAGE_FIELD: &str = "image";
const MASK_FIELD: &str = "mask";
const POLICY_STATUS: u16 = 400;
const POLICY_ERROR_TYPE: &str = "invalid_request_error";
const UNSUPPORTED_MEDIA_URL: &str = "unsupported_media_url";

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error(transparent)]
    Bfl(Box<BflError>),
    #[error("the {field} media must be image bytes, a list of images or a file handle")]
    UnsupportedMedia { field: &'static str },
    #[error("the {field} media is a URL, and this build does not download request media")]
    UnsupportedMediaUrl { field: &'static str },
    #[error("the {field} media nests deeper than the configured recursion limit")]
    MediaDepth { field: &'static str },
    #[error("the edit reply could not be read: {source}")]
    Reply {
        status: u16,
        #[source]
        source: Box<BflError>,
    },
}

impl EditError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Bfl(source) => source.status(),
            Self::Reply { status, .. } => Some(*status),
            Self::UnsupportedMediaUrl { .. } => Some(POLICY_STATUS),
            Self::UnsupportedMedia { .. } | Self::MediaDepth { .. } => None,
        }
    }

    #[must_use]
    pub const fn error_type(&self) -> Option<&'static str> {
        if let Self::UnsupportedMediaUrl { .. } = self {
            return Some(POLICY_ERROR_TYPE);
        }
        None
    }

    #[must_use]
    pub const fn param(&self) -> Option<&'static str> {
        if let Self::UnsupportedMediaUrl { field } = self {
            return Some(*field);
        }
        None
    }

    #[must_use]
    pub const fn code(&self) -> Option<&'static str> {
        if let Self::UnsupportedMediaUrl { .. } = self {
            return Some(UNSUPPORTED_MEDIA_URL);
        }
        None
    }
}

#[must_use]
pub fn map_openai_params(
    image_edit_optional_params: &ImageParams,
    _model: &str,
    _drop_params: bool,
) -> ImageParams {
    let mut optional_params = ImageParams::new();
    for parameter in REQUEST_PARAMS {
        if let Some(value) = image_edit_optional_params.get(parameter)
            && !matches!(value, Json::Null)
        {
            optional_params.insert(parameter.to_owned(), value.clone());
        }
    }
    if !optional_params.contains_key("output_format") {
        optional_params.insert(
            "output_format".to_owned(),
            Json::Text(DEFAULT_OUTPUT_FORMAT.to_owned()),
        );
    }
    optional_params
}

pub fn edit_endpoint(model: &str) -> Result<&'static str, BflError> {
    let name = model.rsplit('/').next().unwrap_or_default().to_lowercase();
    for (candidate, endpoint) in IMAGE_EDIT_MODELS {
        if candidate == name {
            return Ok(endpoint);
        }
    }
    Err(BflError::UnknownModel { model: name })
}

pub fn complete_url(
    api_base: Option<&str>,
    model: &str,
    configured_base: &str,
) -> Result<String, BflError> {
    let base = api_base
        .filter(|base| !base.is_empty())
        .unwrap_or(configured_base)
        .trim_end_matches('/');
    Ok(format!("{base}{}", edit_endpoint(model)?))
}

pub fn transform_image_edit_request(
    prompt: Option<&str>,
    image: Option<&mut SuppliedMedia>,
    mask: MaskInput<'_>,
    image_edit_optional_params: &ImageParams,
    max_depth: i64,
) -> Result<ImageEditForm, EditError> {
    let mut data = ImageParams::new();
    data.insert(
        "prompt".to_owned(),
        prompt.map_or(Json::Null, |text| Json::Text(text.to_owned())),
    );
    let Some(media) = image else {
        return Err(EditError::UnsupportedMedia { field: IMAGE_FIELD });
    };
    let image_bytes = read_media_bytes(media, IMAGE_FIELD, 0, max_depth)?;
    data.insert(
        "input_image".to_owned(),
        Json::Text(BASE64.encode(image_bytes)),
    );
    for (parameter, value) in image_edit_optional_params {
        if REQUEST_PARAMS.contains(&parameter.as_str()) && !matches!(value, Json::Null) {
            data.insert(parameter.clone(), value.clone());
        }
    }
    match mask {
        MaskInput::Absent => {}
        MaskInput::Null => return Err(EditError::UnsupportedMedia { field: MASK_FIELD }),
        MaskInput::Media(mask_media) => {
            let mask_bytes = read_media_bytes(mask_media, MASK_FIELD, 0, max_depth)?;
            data.insert("mask".to_owned(), Json::Text(BASE64.encode(mask_bytes)));
        }
    }
    Ok(ImageEditForm {
        data,
        files: ImageFiles::new(),
    })
}

// ast-grep-ignore: no-long-names
pub fn async_transform_image_edit_request(
    prompt: Option<&str>,
    image: Option<&mut SuppliedMedia>,
    mask: MaskInput<'_>,
    image_edit_optional_params: &ImageParams,
    max_depth: i64,
) -> Result<ImageEditForm, EditError> {
    // LiteLLM's async override downloads the first-image candidate before transforming; this build refuses it instead, so nothing here awaits
    if let Some(media) = image.as_deref()
        && is_remote_candidate(media)
    {
        return Err(EditError::UnsupportedMediaUrl { field: IMAGE_FIELD });
    }
    if let MaskInput::Media(mask_media) = &mask
        && is_remote_candidate(mask_media)
    {
        return Err(EditError::UnsupportedMediaUrl { field: MASK_FIELD });
    }
    transform_image_edit_request(prompt, image, mask, image_edit_optional_params, max_depth)
}

pub fn transform_image_edit_response(
    status: u16,
    body: &[u8],
    now_seconds: i64,
) -> Result<BflImageResponse, EditError> {
    let reply = decode_bfl_reply(body).map_err(|source| EditError::Reply {
        status,
        source: Box::new(source),
    })?;
    let sample = match &reply.result {
        BflResult::Sample(result) => result.sample.as_deref(),
        BflResult::List(_) | BflResult::Other(_) => None,
    };
    let Some(url) = sample.filter(|url| !url.is_empty()) else {
        return Err(EditError::Bfl(Box::new(BflError::NoImageUrl)));
    };
    Ok(BflImageResponse {
        created: now_seconds,
        data: vec![ImageDatum {
            b64_json: None,
            url: Some(url.to_owned()),
            revised_prompt: None,
            provider_specific_fields: None,
        }],
    })
}
