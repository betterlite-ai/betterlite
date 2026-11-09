use serde::Serialize;

pub trait Choice: Copy + 'static {
    const EXPECTED: &'static str;
    const NAMES: &'static [(&'static str, Self)];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputType {
    Text,
    Image,
    Video,
    Audio,
    TextImage,
    MultiInput,
}

impl InputType {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::TextImage => "text_image",
            Self::MultiInput => "multi_input",
        }
    }
}

impl Choice for InputType {
    const EXPECTED: &'static str =
        "Input should be 'text', 'image', 'video', 'audio', 'text_image' or 'multi_input'";
    const NAMES: &'static [(&'static str, Self)] = &[
        ("text", Self::Text),
        ("image", Self::Image),
        ("video", Self::Video),
        ("audio", Self::Audio),
        ("text_image", Self::TextImage),
        ("multi_input", Self::MultiInput),
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingOption {
    Visual,
    Audio,
    Transcription,
}

impl Choice for EmbeddingOption {
    const EXPECTED: &'static str = "Input should be 'visual', 'audio' or 'transcription'";
    const NAMES: &'static [(&'static str, Self)] = &[
        ("visual", Self::Visual),
        ("audio", Self::Audio),
        ("transcription", Self::Transcription),
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingType {
    SeparateEmbedding,
    FusedEmbedding,
}

impl Choice for EmbeddingType {
    const EXPECTED: &'static str = "Input should be 'separate_embedding' or 'fused_embedding'";
    const NAMES: &'static [(&'static str, Self)] = &[
        ("separate_embedding", Self::SeparateEmbedding),
        ("fused_embedding", Self::FusedEmbedding),
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingScope {
    Clip,
    Asset,
}

impl Choice for EmbeddingScope {
    const EXPECTED: &'static str = "Input should be 'clip' or 'asset'";
    const NAMES: &'static [(&'static str, Self)] = &[("clip", Self::Clip), ("asset", Self::Asset)];
}
