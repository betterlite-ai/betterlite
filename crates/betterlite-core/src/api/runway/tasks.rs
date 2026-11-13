pub mod error;
pub mod poll;
pub mod record;
pub mod settings;
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Image,
    Speech,
}

impl TaskKind {
    pub(crate) const fn endpoint(self) -> &'static str {
        match self {
            Self::Image => "v1/text_to_image",
            Self::Speech => "v1/text_to_speech",
        }
    }

    pub(crate) const fn subject(self) -> &'static str {
        match self {
            Self::Image => "RunwayML image generation",
            Self::Speech => "RunwayML TTS",
        }
    }

    pub(crate) const fn polling_subject(self) -> &'static str {
        match self {
            Self::Image => "RunwayML task",
            Self::Speech => "RunwayML TTS task",
        }
    }
}
