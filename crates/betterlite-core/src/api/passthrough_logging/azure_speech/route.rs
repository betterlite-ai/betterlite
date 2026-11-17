use super::{
    BATCH_MODEL, BATCH_PATH_PREFIX, FAST_TRANSCRIPTION_MODEL, FAST_TRANSCRIPTION_PATH, PROVIDER,
    SHORT_AUDIO_MODEL, SHORT_AUDIO_PATH_PREFIX,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeechRoute {
    ShortAudio,
    FastTranscription,
    Batch,
}

// LiteLLM compares rfind positions where an absent prefix is -1, so a short-audio prefix only wins when it comes later.
pub(super) fn classify_route(path: &str) -> SpeechRoute {
    if path.rfind(SHORT_AUDIO_PATH_PREFIX) > path.rfind(BATCH_PATH_PREFIX) {
        return SpeechRoute::ShortAudio;
    }
    if path.ends_with(FAST_TRANSCRIPTION_PATH) {
        return SpeechRoute::FastTranscription;
    }
    SpeechRoute::Batch
}

impl SpeechRoute {
    pub(super) fn model_name(self) -> String {
        match self {
            Self::ShortAudio => format!("{PROVIDER}/{SHORT_AUDIO_MODEL}"),
            Self::FastTranscription => format!("{PROVIDER}/{FAST_TRANSCRIPTION_MODEL}"),
            Self::Batch => format!("{PROVIDER}/{BATCH_MODEL}"),
        }
    }
}
