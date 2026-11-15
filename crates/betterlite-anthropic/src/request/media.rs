use betterlite_prompts::media::parts::{INLINE_REMOTE_IMAGE_URLS, RemoteMedia};

const HTTP_PREFIX: &str = "http://";

// LiteLLM inlines only plain-HTTP image URLs; an HTTPS descriptor travels to the provider as a URL source
#[must_use]
pub fn inlines_remote_media(media: &RemoteMedia) -> bool {
    let selected = INLINE_REMOTE_IMAGE_URLS(media);
    let plain_http = media.url.starts_with(HTTP_PREFIX);
    selected && plain_http
}

pub const INLINE_HTTP_IMAGE_URLS: fn(&RemoteMedia) -> bool = inlines_remote_media;
