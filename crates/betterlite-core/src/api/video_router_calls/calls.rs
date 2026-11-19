use async_trait::async_trait;
use betterlite_error::error::ErrorClass;
use betterlite_error::proxy_model_not_found::{ProxyModelNotFoundError, route_endpoint};

use super::types::{VideoCallArgs, VideoCallKind, VideoReply};

#[derive(Debug, thiserror::Error)]
pub enum VideoRouterError {
    #[error(transparent)]
    NotFound(#[from] ProxyModelNotFoundError),
    #[error("{message}")]
    Mapped { class: ErrorClass, message: String },
}

impl VideoRouterError {
    #[must_use]
    pub fn not_found(kind: VideoCallKind, model: &str) -> Self {
        let name = kind.name();
        Self::NotFound(ProxyModelNotFoundError::new(
            route_endpoint(name).unwrap_or(name),
            model,
            true,
        ))
    }
}

#[async_trait]
pub trait VideoRouterCalls: Send + Sync {
    async fn avideo_generation(
        &self,
        model: &str,
        args: VideoCallArgs,
    ) -> Result<VideoReply, VideoRouterError>;

    async fn avideo_list(
        &self,
        model: &str,
        args: VideoCallArgs,
    ) -> Result<VideoReply, VideoRouterError>;

    async fn avideo_status(
        &self,
        model: &str,
        args: VideoCallArgs,
    ) -> Result<VideoReply, VideoRouterError>;

    async fn avideo_content(
        &self,
        model: &str,
        args: VideoCallArgs,
    ) -> Result<VideoReply, VideoRouterError>;

    async fn avideo_remix(
        &self,
        model: &str,
        args: VideoCallArgs,
    ) -> Result<VideoReply, VideoRouterError>;

    async fn avideo_create_character(
        &self,
        model: &str,
        args: VideoCallArgs,
    ) -> Result<VideoReply, VideoRouterError>;

    async fn avideo_get_character(
        &self,
        model: &str,
        args: VideoCallArgs,
    ) -> Result<VideoReply, VideoRouterError>;

    async fn avideo_edit(
        &self,
        model: &str,
        args: VideoCallArgs,
    ) -> Result<VideoReply, VideoRouterError>;

    async fn avideo_extension(
        &self,
        model: &str,
        args: VideoCallArgs,
    ) -> Result<VideoReply, VideoRouterError>;
}
