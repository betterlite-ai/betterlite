use crate::gemini_media::GEMINI_FILES_API_URI_PREFIX;
use crate::gemini_media::mime::MimeHints;
use betterlite_prompts::media_metadata::url_mime::image_mime_type_from_url;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoogleProvider {
    AiStudio,
    Vertex,
}
#[must_use]
pub fn inlines_remote(provider: GoogleProvider, url: &str, hints: &MimeHints) -> bool {
    let remote = url.starts_with("http://") || url.starts_with("https://");
    match provider {
        GoogleProvider::AiStudio => remote && !url.starts_with(GEMINI_FILES_API_URI_PREFIX),
        GoogleProvider::Vertex => {
            remote
                && !url.starts_with(GEMINI_FILES_API_URI_PREFIX)
                && (url.starts_with("http://")
                    || (hints.explicit().is_none() && image_mime_type_from_url(url).is_none()))
        }
    }
}
