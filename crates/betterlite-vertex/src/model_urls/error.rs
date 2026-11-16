use betterlite_core::api::vertex_location::VertexLocationError;

use super::UrlMode;

#[derive(Debug, thiserror::Error)]
pub enum VertexUrlError {
    #[error("vertex_location is required")]
    LocationRequired,
    #[error("invalid vertex_location format")]
    InvalidLocation,
    #[error("vertex_project is required")]
    ProjectRequired,
    #[error("unable to get vertex url/endpoint for mode: {}", .mode.as_str())]
    UnsupportedMode { mode: UrlMode },
    #[error("the gemini/ route does not support image generation")]
    GeminiImageGeneration,
    #[error(transparent)]
    Target(VertexLocationError),
}

// a location failure keeps the variant callers already match on; a project failure is wrapped whole
impl From<VertexLocationError> for VertexUrlError {
    fn from(error: VertexLocationError) -> Self {
        match error {
            VertexLocationError::Required => Self::LocationRequired,
            VertexLocationError::Invalid => Self::InvalidLocation,
            project @ VertexLocationError::Project { .. } => Self::Target(project),
        }
    }
}
