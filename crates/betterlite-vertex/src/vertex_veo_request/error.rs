use std::io;

use betterlite_core::api::vertex_location::VertexLocationError;
use thiserror::Error;

use crate::model_urls::error::VertexUrlError;
use crate::token_cache::error::CredentialCacheError;

#[derive(Debug, Error)]
pub enum VeoRequestError {
    #[error(
        "vertex_project is required for Vertex AI video generation. Set it via environment variable VERTEXAI_PROJECT or pass as parameter."
    )]
    MissingProject,
    #[error("the vertex_project cannot be used in a URL")]
    Project(#[source] VertexLocationError),
    #[error("the vertex location is not valid")]
    Location(#[source] VertexUrlError),
    #[error("the Vertex AI access token could not be minted")]
    Mint(#[source] CredentialCacheError),
    #[error(
        "Unsupported image value '{value}'. Provide a GCS URI (gs://...), a dict with 'gcsUri' or 'bytesBase64Encoded'/'mimeType', or a binary file-like object."
    )]
    UnsupportedImage { value: String },
    #[error("the image option must be a dict, a gs:// string or a file-like object")]
    ImageShape,
    #[error("the image could not be read")]
    ImageRead(#[source] io::Error),
    #[error("a file-like image cannot become a request parameter")]
    ImageInParameters,
    #[error(
        "prefetched_source_data is required for Vertex AI video edit. Ensure get_video_edit_prefetch_params is called by the handler."
    )]
    PrefetchRequired,
    #[error("Source video generation is not complete yet. Check the video status before editing.")]
    SourceNotDone,
    #[error("No videos found in the completed operation. Cannot edit.")]
    NoVideos,
    #[error("Source video has neither gcsUri nor bytesBase64Encoded. Cannot edit.")]
    MissingSourceVideo,
}
