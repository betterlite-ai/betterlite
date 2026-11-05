pub mod auth;
pub mod error;
pub mod poll;
pub mod record;
pub mod settings;
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    Image,
    Speech,
}

impl TaskKind {
    pub(crate) fn endpoint(self) -> &'static str {
        match self {
            Self::Image => "v1/text_to_image",
            Self::Speech => "v1/text_to_speech",
        }
    }

    pub(crate) fn subject(self) -> &'static str {
        match self {
            Self::Image => "RunwayML image generation",
            Self::Speech => "RunwayML TTS",
        }
    }

    pub(crate) fn polling_subject(self) -> &'static str {
        match self {
            Self::Image => "RunwayML task",
            Self::Speech => "RunwayML TTS task",
        }
    }
}
