use betterlite_openai::api::file_type_records::{
    extension_from_mime, find_type, gemini_accepts, mime_from_type,
};
use serde::Deserialize;

use crate::gemini_media::error::MediaError;

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
        for hint in [&self.format, &self.mime_type, &self.content_type] {
            if let Some(value) = hint.as_deref().filter(|value| !value.is_empty()) {
                return Some(value);
            }
        }
        None
    }
}

#[must_use]
pub fn apply_mime_alias(mime_type: &str) -> String {
    let normalized = mime_type
        .split_once(';')
        .map_or(mime_type, |(media_type, _parameters)| media_type)
        .trim()
        .to_lowercase();
    for (alias, canonical) in GEMINI_MIME_TYPE_ALIASES {
        if *alias == normalized {
            return (*canonical).to_owned();
        }
    }
    normalized
}

pub fn normalize_mime_type(mime_type: &str) -> Result<String, MediaError> {
    let normalized = apply_mime_alias(mime_type);
    let extension =
        extension_from_mime(&normalized).map_err(|source| MediaError::UnsupportedMediaType {
            subject: normalized.clone(),
            source: Some(source),
        })?;
    let Some(file_type) = find_type(extension) else {
        return Err(MediaError::UnsupportedMediaType {
            subject: normalized,
            source: None,
        });
    };
    if !gemini_accepts(file_type) {
        return Err(MediaError::UnsupportedMediaType {
            subject: mime_from_type(file_type).to_owned(),
            source: None,
        });
    }
    Ok(mime_from_type(file_type).to_owned())
}
