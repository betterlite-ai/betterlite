use betterlite_prompts::media::data_url::{MAX_MEDIA_BYTES, decode_data_url};
use serde::Deserialize;

use crate::image::dimensions::image_dimensions;
use crate::image::error::ImageError;
use crate::image::geometry::{resize_high_res, tiles_needed};
use crate::image::settings::Settings;

pub const DEFAULT_BASE_TOKENS: u64 = 85;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Low,
    High,
    #[default]
    Auto,
}

pub fn calculate_img_tokens(
    data: &str,
    mode: Mode,
    base_tokens: u64,
    use_default_image_token_count: bool,
    settings: &Settings,
) -> Result<u64, ImageError> {
    if use_default_image_token_count {
        return Ok(settings.default_token_count);
    }
    match mode {
        Mode::Low | Mode::Auto => Ok(base_tokens),
        Mode::High => {
            let bytes = decode_data_url(data, MAX_MEDIA_BYTES)?;
            let (width, height) = image_dimensions(&bytes, settings)?;
            let (resized_width, resized_height) = resize_high_res(width, height, settings)?;
            let tiles = tiles_needed(resized_width, resized_height, settings)?;
            let tile_tokens = base_tokens
                .checked_mul(2)
                .and_then(|doubled| doubled.checked_mul(tiles))
                .ok_or(ImageError::Overflow)?;
            base_tokens
                .checked_add(tile_tokens)
                .ok_or(ImageError::Overflow)
        }
    }
}
