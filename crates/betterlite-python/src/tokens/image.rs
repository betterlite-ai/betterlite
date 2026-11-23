use pyo3::prelude::*;

use betterlite_prompts::media::data_url::{MAX_MEDIA_BYTES, decode_data_url};
use betterlite_tokenizer::image::dimensions::image_dimensions as dimensions_of;
use betterlite_tokenizer::image::error::ImageError;
use betterlite_tokenizer::image::geometry::{
    high_detail_upper_bound, resize_high_res, tiles_needed,
};
use betterlite_tokenizer::image::header::image_type as header_type;
use betterlite_tokenizer::image::settings::Settings;
use betterlite_tokenizer::image::tokens::{DEFAULT_BASE_TOKENS, calculate_img_tokens};

use super::convert::{
    BoundRequest, DimensionsRequest, ImageTokensRequest, ResizeRequest, TileRequest, read,
};
use super::error::TokensError;

#[pyfunction]
pub(crate) fn image_tokens(request: &str) -> PyResult<u64> {
    let wire: ImageTokensRequest = read(request)?;
    let tokens = calculate_img_tokens(
        &wire.data,
        wire.mode,
        wire.base_tokens.unwrap_or(DEFAULT_BASE_TOKENS),
        wire.use_default_image_token_count,
        &Settings::default(),
    )
    .map_err(TokensError::from)?;
    Ok(tokens)
}

#[pyfunction]
pub(crate) fn image_tile_count(request: &str) -> PyResult<u64> {
    let wire: TileRequest = read(request)?;
    let tiles = tiles_needed(wire.resized_width, wire.resized_height, &wire.settings())
        .map_err(TokensError::from)?;
    Ok(tiles)
}

#[pyfunction]
pub(crate) fn image_type(data: &[u8]) -> Option<&'static str> {
    if let Some(kind) = header_type(data) {
        return Some(kind.as_str());
    }
    None
}

#[pyfunction]
pub(crate) fn image_dimensions(request: &str) -> PyResult<(u32, u32)> {
    let wire: DimensionsRequest = read(request)?;
    let bytes = decode_data_url(&wire.data, MAX_MEDIA_BYTES).map_err(ImageError::from)?;
    let (width, height) = dimensions_of(&bytes, &Settings::default()).map_err(TokensError::from)?;
    Ok((width, height))
}

#[pyfunction]
pub(crate) fn image_resize(request: &str) -> PyResult<(u32, u32)> {
    let wire: ResizeRequest = read(request)?;
    let (width, height) = resize_high_res(wire.width, wire.height, &Settings::default())
        .map_err(TokensError::from)?;
    Ok((width, height))
}

#[pyfunction]
pub(crate) fn image_high_detail_bound(request: &str) -> PyResult<u64> {
    let wire: BoundRequest = read(request)?;
    let base_tokens = wire.base_tokens.unwrap_or(DEFAULT_BASE_TOKENS);
    let bound =
        high_detail_upper_bound(base_tokens, &Settings::default()).map_err(TokensError::from)?;
    Ok(bound)
}

mod tests;
