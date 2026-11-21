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
    const ALL: [Self; 14] = [
        Self::Chat,
        Self::Completion,
        Self::Embedding,
        Self::AudioSpeech,
        Self::AudioTranscription,
        Self::ImageGeneration,
        Self::ImageEdit,
        Self::VideoGeneration,
        Self::Rerank,
        Self::Realtime,
        Self::Batch,
        Self::Responses,
        Self::Ocr,
        Self::Evaluation,
    ];
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        for mode in Self::ALL {
            if mode.name() == name {
                return Some(mode);
            }
        }
        None
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
