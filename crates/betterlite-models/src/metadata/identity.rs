use serde::{Deserialize, Serialize};

/// Primary API surface the model serves, per the pinned schema's `mode` enum.
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

/// Input or output modality. The schema allows `code` only on output, but a
/// shared enum is still exact: no pinned entry ever puts `code` on input.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    Text,
    Image,
    Audio,
    Video,
    Code,
}

/// Audio container format the model can return.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioFormat {
    Mp3,
    Wav,
}
