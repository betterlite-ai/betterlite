use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_vertex::gemini_media::mime::MimeHints;
use betterlite_vertex::gemini_media::part::VideoMetadata;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    MimeAlias { values: Vec<String> },
    ExplicitGcsFormat { cases: Vec<MediaCase> },
    FilesUriWithoutMime { cases: Vec<MediaCase> },
    HttpsPassthrough { cases: Vec<MediaCase> },
    VideoOffsets { cases: Vec<MediaCase> },
    ResolutionMaximum { messages: Vec<Message> },
    PartKeyEquivalence { cases: Vec<EquivalenceCase> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MediaCase {
    pub(super) url: String,
    #[serde(default)]
    pub(super) hints: MimeHints,
    #[serde(default)]
    pub(super) detail: Option<String>,
    #[serde(default)]
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) video_metadata: Option<VideoMetadata>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EquivalenceCase {
    pub(super) parts: Vec<Object>,
    pub(super) part: Object,
    #[serde(default)]
    pub(super) excluded_keys: Vec<String>,
}
