use betterlite_prompts::media::data_url::{MAX_MEDIA_BYTES, decode_data_url};
use betterlite_tokenizer::image::dimensions::image_dimensions;
use betterlite_tokenizer::image::error::ImageError;
use betterlite_tokenizer::image::geometry::{
    high_detail_upper_bound, resize_high_res, tiles_needed,
};
use betterlite_tokenizer::image::header::{ImageType, image_type};
use betterlite_tokenizer::image::settings::Settings;
use betterlite_tokenizer::image::tokens::{DEFAULT_BASE_TOKENS, Mode, calculate_img_tokens};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    settings: Settings,
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    CalculateImgTokens {
        data: String,
        mode: Option<Mode>,
        base_tokens: Option<u64>,
        use_default_image_token_count: Option<bool>,
    },
    ResizeImageHighRes {
        width: u32,
        height: u32,
    },
    CalculateTilesNeeded {
        resized_width: u32,
        resized_height: u32,
    },
    HighDetailImageTokenUpperBound {
        base_tokens: Option<u64>,
    },
    GetImageType {
        data: String,
    },
    GetImageDimensions {
        data: String,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let values = input
            .calls
            .iter()
            .map(|call| run_call(call, &input.settings))
            .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
        Ok(Value::Array(values))
    }
}

fn run_call(call: &Call, settings: &Settings) -> Result<Value, Box<HelperError>> {
    match call {
        Call::CalculateImgTokens {
            data,
            mode,
            base_tokens,
            use_default_image_token_count,
        } => count_img_tokens(
            data,
            *mode,
            *base_tokens,
            *use_default_image_token_count,
            settings,
        ),
        Call::ResizeImageHighRes { width, height } => {
            let (resized_width, resized_height) =
                resize_high_res(*width, *height, settings).map_err(|error| failure(&error))?;
            Ok(json!([resized_width, resized_height]))
        }
        Call::CalculateTilesNeeded {
            resized_width,
            resized_height,
        } => {
            let tiles = tiles_needed(*resized_width, *resized_height, settings)
                .map_err(|error| failure(&error))?;
            Ok(json!(tiles))
        }
        Call::HighDetailImageTokenUpperBound { base_tokens } => {
            let tokens =
                high_detail_upper_bound(base_tokens.unwrap_or(DEFAULT_BASE_TOKENS), settings)
                    .map_err(|error| failure(&error))?;
            Ok(json!(tokens))
        }
        Call::GetImageType { data } => image_type_of(data),
        Call::GetImageDimensions { data } => dimensions_of(data, settings),
    }
}

fn image_type_of(data: &str) -> Result<Value, Box<HelperError>> {
    let bytes = decode_data_url(data, MAX_MEDIA_BYTES)
        .map_err(|source| failure(&ImageError::from(source)))?;
    Ok(json!(image_type(&bytes).map(ImageType::as_str)))
}

fn dimensions_of(data: &str, settings: &Settings) -> Result<Value, Box<HelperError>> {
    let bytes = decode_data_url(data, MAX_MEDIA_BYTES)
        .map_err(|source| failure(&ImageError::from(source)))?;
    let (width, height) = image_dimensions(&bytes, settings).map_err(|error| failure(&error))?;
    Ok(json!([width, height]))
}

fn count_img_tokens(
    data: &str,
    mode: Option<Mode>,
    base_tokens: Option<u64>,
    use_default_image_token_count: Option<bool>,
    settings: &Settings,
) -> Result<Value, Box<HelperError>> {
    let tokens = calculate_img_tokens(
        data,
        mode.unwrap_or_default(),
        base_tokens.unwrap_or(DEFAULT_BASE_TOKENS),
        use_default_image_token_count.unwrap_or_default(),
        settings,
    )
    .map_err(|error| failure(&error))?;
    Ok(json!(tokens))
}

fn failure(error: &ImageError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ImageError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

mod tests;
