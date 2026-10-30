use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    AudioSpeech,
    AudioTranscription,
    Chat,
    Completion,
    Embedding,
    Evaluation,
    Guardrail,
    ImageEdit,
    ImageGeneration,
    Moderation,
    Ocr,
    Realtime,
    Rerank,
    Responses,
    Search,
    VectorStore,
    VideoGeneration,
}

// code appears only on output in the pinned schema, but one enum covers both directions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    Text,
    Image,
    Audio,
    Video,
    Code,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioFormat {
    Mp3,
    Wav,
}
