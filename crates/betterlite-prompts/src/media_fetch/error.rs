use thiserror::Error;

use crate::media_metadata::error::MediaMetadataError;

// Every variant is LiteLLM's ImageFetchError, which is a BadRequestError, so the caller answers 400
#[derive(Debug, Error)]
pub enum ImageFetchError {
    #[error("Error: Image URL download is disabled (MAX_IMAGE_URL_DOWNLOAD_SIZE_MB=0). url={url}")]
    Disabled { url: String },
    #[error(
        "Error: Unable to fetch image from URL. The proxy could not resolve this host or its URL policy rejected it; an admin can check the proxy log and `user_url_allowed_hosts` in litellm_settings. url={url}"
    )]
    Rejected { url: String },
    #[error("Error: Unable to fetch image from URL after 3 attempts. url={url}")]
    Attempts { url: String },
    #[error("Error: Unable to fetch image from URL. Status code: {status}, url={url}")]
    Status { status: u16, url: String },
    #[error("Error: Image size ({size}MB) exceeds maximum allowed size ({limit}MB). url={url}")]
    TooLarge {
        size: String,
        limit: String,
        url: String,
    },
    #[error(
        "Error: Unable to determine image content type from the server's headers, the URL, or the image bytes. url={url}"
    )]
    UnknownMediaType {
        url: String,
        #[source]
        source: MediaMetadataError,
    },
}
