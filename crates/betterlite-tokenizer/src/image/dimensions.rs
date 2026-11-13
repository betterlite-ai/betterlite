use crate::image::error::ImageError;
use crate::image::header::{ImageType, image_type};
use crate::image::settings::Settings;

pub fn image_dimensions(data: &[u8], settings: &Settings) -> Result<(u32, u32), ImageError> {
    match image_type(data) {
        Some(ImageType::Png) => png_dimensions(data),
        Some(ImageType::Gif) => gif_dimensions(data),
        Some(ImageType::Jpeg) => jpeg_dimensions(data),
        Some(ImageType::Webp) => webp_dimensions(data, settings),
        Some(ImageType::Heic) | None => Ok((settings.default_width, settings.default_height)),
    }
}

fn fixed_bytes<const N: usize>(
    data: &[u8],
    start: usize,
    kind: ImageType,
) -> Result<[u8; N], ImageError> {
    let end = start.checked_add(N).ok_or(ImageError::Truncated { kind })?;
    let slice = data.get(start..end).ok_or(ImageError::Truncated { kind })?;
    <[u8; N]>::try_from(slice).map_err(|_error| ImageError::Truncated { kind })
}

fn png_dimensions(data: &[u8]) -> Result<(u32, u32), ImageError> {
    let width = u32::from_be_bytes(fixed_bytes(data, 16, ImageType::Png)?);
    let height = u32::from_be_bytes(fixed_bytes(data, 20, ImageType::Png)?);
    Ok((width, height))
}

fn gif_dimensions(data: &[u8]) -> Result<(u32, u32), ImageError> {
    let width = u16::from_le_bytes(fixed_bytes(data, 6, ImageType::Gif)?);
    let height = u16::from_le_bytes(fixed_bytes(data, 8, ImageType::Gif)?);
    Ok((u32::from(width), u32::from(height)))
}

fn webp_dimensions(data: &[u8], settings: &Settings) -> Result<(u32, u32), ImageError> {
    let chunk = data.get(12..16);
    if chunk == Some(&b"VP8X"[..]) {
        return vp8x_dimensions(data);
    }
    if chunk == Some(&b"VP8 "[..]) {
        return vp8_dimensions(data);
    }
    if chunk == Some(&b"VP8L"[..]) {
        return vp8l_dimensions(data);
    }
    Ok((settings.default_width, settings.default_height))
}

fn compact_dimension(bytes: [u8; 3]) -> Result<u32, ImageError> {
    let [first, second, third] = bytes;
    let value = u32::from_le_bytes([first, second, third, 0]);
    value.checked_add(1).ok_or(ImageError::Overflow)
}

fn vp8x_dimensions(data: &[u8]) -> Result<(u32, u32), ImageError> {
    let width = compact_dimension(fixed_bytes(data, 24, ImageType::Webp)?)?;
    let height = compact_dimension(fixed_bytes(data, 27, ImageType::Webp)?)?;
    Ok((width, height))
}

fn vp8_dimensions(data: &[u8]) -> Result<(u32, u32), ImageError> {
    let width = u16::from_le_bytes(fixed_bytes(data, 26, ImageType::Webp)?) & 0x3FFF;
    let height = u16::from_le_bytes(fixed_bytes(data, 28, ImageType::Webp)?) & 0x3FFF;
    Ok((u32::from(width), u32::from(height)))
}

fn vp8l_dimensions(data: &[u8]) -> Result<(u32, u32), ImageError> {
    let bits = u32::from_le_bytes(fixed_bytes(data, 21, ImageType::Webp)?);
    let width = (bits & 0x3FFF).checked_add(1).ok_or(ImageError::Overflow)?;
    let shifted = bits.checked_shr(14).ok_or(ImageError::Overflow)?;
    let height = (shifted & 0x3FFF)
        .checked_add(1)
        .ok_or(ImageError::Overflow)?;
    Ok((width, height))
}

fn jpeg_dimensions(data: &[u8]) -> Result<(u32, u32), ImageError> {
    let kind = ImageType::Jpeg;
    let mut position = 2_usize;
    loop {
        let mut marker = *data.get(position).ok_or(ImageError::Truncated { kind })?;
        position = position.checked_add(1).ok_or(ImageError::Overflow)?;
        while marker == 0xFF {
            marker = *data.get(position).ok_or(ImageError::Truncated { kind })?;
            position = position.checked_add(1).ok_or(ImageError::Overflow)?;
        }
        let length = u16::from_be_bytes(fixed_bytes(data, position, kind)?);
        position = position.checked_add(2).ok_or(ImageError::Overflow)?;
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let precision = position.checked_add(1).ok_or(ImageError::Overflow)?;
            let height = u16::from_be_bytes(fixed_bytes(data, precision, kind)?);
            let width_at = precision.checked_add(2).ok_or(ImageError::Overflow)?;
            let width = u16::from_be_bytes(fixed_bytes(data, width_at, kind)?);
            return Ok((u32::from(width), u32::from(height)));
        }
        let size = i64::from(length)
            .checked_sub(2)
            .ok_or(ImageError::Overflow)?;
        if size.is_negative() {
            let backward =
                usize::try_from(size.unsigned_abs()).map_err(|_error| ImageError::Overflow)?;
            position = position.checked_sub(backward).ok_or(ImageError::Overflow)?;
        } else {
            let forward = usize::try_from(size).map_err(|_error| ImageError::Overflow)?;
            position = position.checked_add(forward).ok_or(ImageError::Overflow)?;
        }
    }
}
