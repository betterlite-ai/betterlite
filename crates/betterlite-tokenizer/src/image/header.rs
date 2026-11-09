const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
const GIF_PREFIX: &[u8] = b"GIF8";
const GIF_VERSION_BYTE: u8 = b'a';
const JPEG_MAGIC: &[u8] = b"\xff\xd8\xff";
const HEIC_BRAND: &[u8] = b"ftyp";
const RIFF_MAGIC: &[u8] = b"RIFF";
const WEBP_MAGIC: &[u8] = b"WEBP";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageType {
    Png,
    Gif,
    Jpeg,
    Heic,
    Webp,
}

impl ImageType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Gif => "gif",
            Self::Jpeg => "jpeg",
            Self::Heic => "heic",
            Self::Webp => "webp",
        }
    }
}

// LiteLLM checks these signatures in this order and needs no more than 12 bytes
#[must_use]
pub fn image_type(data: &[u8]) -> Option<ImageType> {
    if data.starts_with(PNG_MAGIC) {
        return Some(ImageType::Png);
    }
    if data.starts_with(GIF_PREFIX) && data.get(5) == Some(&GIF_VERSION_BYTE) {
        return Some(ImageType::Gif);
    }
    if data.starts_with(JPEG_MAGIC) {
        return Some(ImageType::Jpeg);
    }
    if data.get(4..8) == Some(HEIC_BRAND) {
        return Some(ImageType::Heic);
    }
    if data.starts_with(RIFF_MAGIC) && data.get(8..12) == Some(WEBP_MAGIC) {
        return Some(ImageType::Webp);
    }
    None
}
