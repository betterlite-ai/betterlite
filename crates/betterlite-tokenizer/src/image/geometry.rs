use crate::image::error::ImageError;
use crate::image::settings::Settings;

// Python truncates a binary64 quotient; integer division scores one pixel higher on some exact-integer aspects, recorded in docs/deviations/B258.yaml
pub fn resize_high_res(
    width: u32,
    height: u32,
    settings: &Settings,
) -> Result<(u32, u32), ImageError> {
    if width <= settings.max_short_side && height <= settings.max_short_side {
        return Ok((width, height));
    }
    let longer = u64::from(width.max(height));
    let shorter = u64::from(width.min(height));
    if shorter == 0 {
        return Err(ImageError::ZeroSide);
    }
    let short_limit = u64::from(settings.max_short_side);
    let long_limit = u64::from(settings.max_long_side);
    let scaled = short_limit
        .checked_mul(longer)
        .ok_or(ImageError::Overflow)?;
    let long_side = scaled.checked_div(shorter).ok_or(ImageError::ZeroSide)?;
    if long_side > long_limit {
        let clamped = long_limit
            .checked_mul(shorter)
            .ok_or(ImageError::Overflow)?;
        let clamped_short = clamped.checked_div(longer).ok_or(ImageError::ZeroSide)?;
        return scaled_pair(width, height, clamped_short, long_limit);
    }
    scaled_pair(width, height, short_limit, long_side)
}

fn scaled_pair(width: u32, height: u32, short: u64, long: u64) -> Result<(u32, u32), ImageError> {
    let short_side = u32::try_from(short).map_err(|_error| ImageError::Overflow)?;
    let long_side = u32::try_from(long).map_err(|_error| ImageError::Overflow)?;
    if width <= height {
        Ok((short_side, long_side))
    } else {
        Ok((long_side, short_side))
    }
}

pub fn tiles_needed(
    resized_width: u32,
    resized_height: u32,
    settings: &Settings,
) -> Result<u64, ImageError> {
    let across = tiles_across(u64::from(resized_width), u64::from(settings.tile_width))?;
    let down = tiles_across(u64::from(resized_height), u64::from(settings.tile_height))?;
    across.checked_mul(down).ok_or(ImageError::Overflow)
}

fn tiles_across(side: u64, tile: u64) -> Result<u64, ImageError> {
    let padding = tile.checked_sub(1).ok_or(ImageError::ZeroTile)?;
    let padded = side.checked_add(padding).ok_or(ImageError::Overflow)?;
    padded.checked_div(tile).ok_or(ImageError::ZeroTile)
}

pub fn high_detail_upper_bound(base_tokens: u64, settings: &Settings) -> Result<u64, ImageError> {
    let largest = tiles_needed(settings.max_long_side, settings.max_short_side, settings)?;
    let doubled = base_tokens.checked_mul(2).ok_or(ImageError::Overflow)?;
    let tile_tokens = doubled.checked_mul(largest).ok_or(ImageError::Overflow)?;
    base_tokens
        .checked_add(tile_tokens)
        .ok_or(ImageError::Overflow)
}
