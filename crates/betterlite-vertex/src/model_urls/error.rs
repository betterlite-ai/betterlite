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
}
