use serde_json::Value;

use crate::api::audio_file_input::ProcessedAudioFile;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 5] = [
    "language",
    "prompt",
    "response_format",
    "temperature",
    "timestamp_granularities",
];

pub const AUDIO_PATH: &str = "/ml/v1/audio/transcriptions";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatsonxTranscriptionRequest {
    pub fields: Vec<(String, Value)>,
    pub file: ProcessedAudioFile,
}
