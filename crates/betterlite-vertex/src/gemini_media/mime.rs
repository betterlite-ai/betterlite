use crate::gemini_media::error::MediaError;
use betterlite_openai::api::file_type_records::{
    extension_from_mime, find_type, gemini_accepts, mime_from_type,
};
use serde::Deserialize;
const GEMINI_MIME_TYPE_ALIASES: &[(&str, &str)] = &[("image/jpg", "image/jpeg")];
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MimeHints {
    pub format: Option<String>,
    pub mime_type: Option<String>,
    pub content_type: Option<String>,
}
impl MimeHints {
    #[must_use]
    pub fn explicit(&self) -> Option<&str> {
        [&self.format, &self.mime_type, &self.content_type]
            .into_iter()
            .find_map(|hint| hint.as_deref().filter(|value| !value.is_empty()))
    }
}
#[must_use]
pub fn apply_mime_alias(mime_type: &str) -> String {
    let media_type = mime_type
        .split_once(';')
        .map_or(mime_type, |(head, _parameters)| head);
    let normalized = media_type.trim().to_lowercase();
    GEMINI_MIME_TYPE_ALIASES
        .iter()
        .find_map(|(alias, canonical)| (*alias == normalized).then_some(*canonical))
        .map(str::to_owned)
        .unwrap_or(normalized)
}
pub fn normalize_mime_type(mime_type: &str) -> Result<String, MediaError> {
    let normalized = apply_mime_alias(mime_type);
    let extension = match extension_from_mime(&normalized) {
        Ok(extension) => extension,
        Err(source) => {
            return Err(MediaError::UnsupportedMediaType(normalized, Some(source)));
        }
    };
    let Some(file_type) = find_type(extension) else {
        return Err(MediaError::UnsupportedMediaType(normalized, None));
    };
    if !gemini_accepts(file_type) {
        return Err(MediaError::UnsupportedMediaType(
            mime_from_type(file_type).to_owned(),
            None,
        ));
    }
    Ok(mime_from_type(file_type).to_owned())
}
