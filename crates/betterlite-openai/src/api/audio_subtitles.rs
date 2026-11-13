pub mod grouping;
pub mod render;
pub mod timing;
pub mod width;

mod tests;

pub const CUE_MAX_CHARS: usize = 84;
pub const CUE_MAX_DURATION_MS: i64 = 7000;
pub const CUE_GAP_MS: i64 = 700;

pub const SRT_RESPONSE_FORMAT: &str = "srt";
pub const VTT_RESPONSE_FORMAT: &str = "vtt";
pub const SUBTITLE_RESPONSE_FORMATS: [&str; 2] = [SRT_RESPONSE_FORMAT, VTT_RESPONSE_FORMAT];

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(untagged)]
pub enum Speaker {
    Number(i64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct SubtitleToken {
    pub text: String,
    #[serde(default)]
    pub start_ms: Option<i64>,
    #[serde(default)]
    pub end_ms: Option<i64>,
    #[serde(default)]
    pub speaker: Option<Speaker>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    pub text: String,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub speaker: Option<Speaker>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SubtitleCue {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}
