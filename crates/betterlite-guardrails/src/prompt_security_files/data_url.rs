use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::error::FileSanitizationError;

pub struct DataUrl<'url> {
    pub header: &'url str,
    pub mime_type: &'url str,
    pub bytes: Vec<u8>,
}

impl<'url> DataUrl<'url> {
    pub fn parse(url: &'url str) -> Result<Self, FileSanitizationError> {
        let (header, encoded) = url
            .split_once(',')
            .ok_or(FileSanitizationError::DataUrlSeparator)?;
        let mime_type = header
            .split(';')
            .next()
            .and_then(|scheme_and_mime| scheme_and_mime.split(':').nth(1))
            .unwrap_or_default();
        Ok(Self {
            header,
            mime_type,
            bytes: decode_base64(encoded)?,
        })
    }

    #[must_use]
    pub fn image_filename(&self) -> String {
        format!(
            "image.{}",
            self.mime_type.rsplit('/').next().unwrap_or_default()
        )
    }
}

pub fn decode_base64(encoded: &str) -> Result<Vec<u8>, FileSanitizationError> {
    // Python's b64decode drops whitespace inside the payload.
    let compact: String = encoded
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect();
    STANDARD
        .decode(compact)
        .map_err(|source| FileSanitizationError::Base64 { source })
}
