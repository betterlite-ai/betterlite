use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::videos::VideoOptions;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCallKind {
    Generation,
    List,
    Status,
    Content,
    Remix,
    CreateCharacter,
    GetCharacter,
    Edit,
    Extension,
}

impl VideoCallKind {
    // LiteLLM's route_type for each call, the name it feeds to getattr on the router or on litellm
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Generation => "avideo_generation",
            Self::List => "avideo_list",
            Self::Status => "avideo_status",
            Self::Content => "avideo_content",
            Self::Remix => "avideo_remix",
            Self::CreateCharacter => "avideo_create_character",
            Self::GetCharacter => "avideo_get_character",
            Self::Edit => "avideo_edit",
            Self::Extension => "avideo_extension",
        }
    }
}

// every argument the SDK call would receive, named and loose alike, plus the file parts it takes
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoCallArgs {
    pub fields: VideoOptions,
    pub uploads: Vec<VideoUpload>,
    pub specific_deployment: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoUpload {
    pub field: String,
    pub filename: String,
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoReply {
    Json(JsonValue),
    Bytes(Vec<u8>),
}
