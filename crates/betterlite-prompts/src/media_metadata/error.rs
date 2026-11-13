use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MediaMetadataError {
    #[error("file content is a bare string; open the path and pass bytes or a handle instead")]
    BareStringContent,
    #[error("could not determine the content type of url {url}")]
    ContentTypeUndetermined { url: String },
}
