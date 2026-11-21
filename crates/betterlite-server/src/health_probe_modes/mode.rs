#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HealthProbeMode {
    Chat,
    Completion,
    Embedding,
    AudioSpeech,
    AudioTranscription,
    ImageGeneration,
    ImageEdit,
    VideoGeneration,
    Rerank,
    Realtime,
    Batch,
    Responses,
    Ocr,
    Evaluation,
}
impl HealthProbeMode {
    #[must_use]
    pub const fn from_name(name: &str) -> Option<Self> {
        match name {
            "chat" => Some(Self::Chat),
            "completion" => Some(Self::Completion),
            "embedding" => Some(Self::Embedding),
            "audio_speech" => Some(Self::AudioSpeech),
            "audio_transcription" => Some(Self::AudioTranscription),
            "image_generation" => Some(Self::ImageGeneration),
            "image_edit" => Some(Self::ImageEdit),
            "video_generation" => Some(Self::VideoGeneration),
            "rerank" => Some(Self::Rerank),
            "realtime" => Some(Self::Realtime),
            "batch" => Some(Self::Batch),
            "responses" => Some(Self::Responses),
            "ocr" => Some(Self::Ocr),
            "evaluation" => Some(Self::Evaluation),
            _ => None,
        }
    }
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Completion => "completion",
            Self::Embedding => "embedding",
            Self::AudioSpeech => "audio_speech",
            Self::AudioTranscription => "audio_transcription",
            Self::ImageGeneration => "image_generation",
            Self::ImageEdit => "image_edit",
            Self::VideoGeneration => "video_generation",
            Self::Rerank => "rerank",
            Self::Realtime => "realtime",
            Self::Batch => "batch",
            Self::Responses => "responses",
            Self::Ocr => "ocr",
            Self::Evaluation => "evaluation",
        }
    }
}
