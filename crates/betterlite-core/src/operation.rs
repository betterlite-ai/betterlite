pub mod call_types;
pub mod chat;
pub mod container_call;
pub mod embedding;
pub mod image_edit;
pub mod image_generation;
pub mod llm_passthrough;
pub mod moderation;
pub mod responses;
pub mod text_completion;
pub mod text_stream;
pub mod video_content;
pub mod video_create_character;
pub mod video_edit;
pub mod video_extension;
pub mod video_generation;
pub mod video_get_character;
pub mod video_list;
pub mod video_remix;
pub mod video_status;

// every executable call type: a request variant, a result variant and one dispatch module each
#[rustfmt::skip]
pub enum Operation {
    Chat(chat::ChatRequest),
    Embedding(embedding::EmbeddingRequest),
    TextCompletion(text_completion::TextCompletionRequest),
    TextStream(text_completion::TextCompletionRequest),
    Moderation(moderation::ModerationRequest),
    Responses(responses::ResponsesRequest),
    ImageGeneration(image_generation::ImageGenerationRequest),
    ImageEdit(image_edit::ImageEditRequest),
    VideoGeneration(video_generation::VideoGenerationRequest),
    VideoList(video_list::VideoListRequest),
    VideoStatus(video_status::VideoStatusRequest),
    VideoContent(video_content::VideoContentRequest),
    VideoRemix(video_remix::VideoRemixRequest),
    VideoCreateCharacter(video_create_character::VideoCreateCharacterRequest),
    VideoGetCharacter(video_get_character::VideoGetCharacterRequest),
    VideoEdit(video_edit::VideoEditRequest),
    VideoExtension(video_extension::VideoExtensionRequest),
    LlmPassthrough(llm_passthrough::LlmPassthroughRequest),
    ContainerCall(container_call::ContainerCallRequest),
}

impl Operation {
    #[must_use]
    pub const fn entry(&self) -> Entry {
        match self {
            Self::Chat(_) => Entry::Chat,
            Self::Embedding(_) => Entry::Embedding,
            Self::TextCompletion(_) => Entry::TextCompletion,
            Self::TextStream(_) => Entry::TextStream,
            Self::Moderation(_) => Entry::Moderation,
            Self::Responses(_) => Entry::Responses,
            Self::ImageGeneration(_) => Entry::ImageGeneration,
            Self::ImageEdit(_) => Entry::ImageEdit,
            Self::VideoGeneration(_) => Entry::VideoGeneration,
            Self::VideoList(_) => Entry::VideoList,
            Self::VideoStatus(_) => Entry::VideoStatus,
            Self::VideoContent(_) => Entry::VideoContent,
            Self::VideoRemix(_) => Entry::VideoRemix,
            Self::VideoCreateCharacter(_) => Entry::VideoCreateCharacter,
            Self::VideoGetCharacter(_) => Entry::VideoGetCharacter,
            Self::VideoEdit(_) => Entry::VideoEdit,
            Self::VideoExtension(_) => Entry::VideoExtension,
            Self::LlmPassthrough(_) => Entry::LlmPassthrough,
            Self::ContainerCall(_) => Entry::ContainerCall,
        }
    }
}

#[rustfmt::skip]
pub enum CallResult {
    Chat(chat::ChatResult),
    Embedding(embedding::EmbeddingResult),
    TextCompletion(text_completion::TextCompletionResult),
    TextStream(text_stream::TextStreamResult),
    Moderation(moderation::ModerationResult),
    Responses(responses::ResponsesResult),
    ImageGeneration(image_generation::ImageGenerationResult),
    ImageEdit(image_edit::ImageEditResult),
    VideoGeneration(video_generation::VideoGenerationResult),
    VideoList(video_list::VideoListResult),
    VideoStatus(video_status::VideoStatusResult),
    VideoContent(video_content::VideoContentResult),
    VideoRemix(video_remix::VideoRemixResult),
    VideoCreateCharacter(video_create_character::VideoCreateCharacterResult),
    VideoGetCharacter(video_get_character::VideoGetCharacterResult),
    VideoEdit(video_edit::VideoEditResult),
    VideoExtension(video_extension::VideoExtensionResult),
    LlmPassthrough(llm_passthrough::LlmPassthroughResult),
    ContainerCall(container_call::ContainerCallResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Entry {
    Chat,
    Embedding,
    TextCompletion,
    TextStream,
    Moderation,
    Responses,
    ImageGeneration,
    ImageEdit,
    VideoGeneration,
    VideoList,
    VideoStatus,
    VideoContent,
    VideoRemix,
    VideoCreateCharacter,
    VideoGetCharacter,
    VideoEdit,
    VideoExtension,
    LlmPassthrough,
    ContainerCall,
}

impl Entry {
    pub const ALL: [Self; 19] = [
        Self::Chat,
        Self::Embedding,
        Self::TextCompletion,
        Self::TextStream,
        Self::Moderation,
        Self::Responses,
        Self::ImageGeneration,
        Self::ImageEdit,
        Self::VideoGeneration,
        Self::VideoList,
        Self::VideoStatus,
        Self::VideoContent,
        Self::VideoRemix,
        Self::VideoCreateCharacter,
        Self::VideoGetCharacter,
        Self::VideoEdit,
        Self::VideoExtension,
        Self::LlmPassthrough,
        Self::ContainerCall,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Embedding => "embedding",
            Self::TextCompletion => "text_completion",
            Self::TextStream => "text_stream",
            Self::Moderation => "moderation",
            Self::Responses => "responses",
            Self::ImageGeneration => "image_generation",
            Self::ImageEdit => "image_edit",
            Self::VideoGeneration => "video_generation",
            Self::VideoList => "video_list",
            Self::VideoStatus => "video_status",
            Self::VideoContent => "video_content",
            Self::VideoRemix => "video_remix",
            Self::VideoCreateCharacter => "video_create_character",
            Self::VideoGetCharacter => "video_get_character",
            Self::VideoEdit => "video_edit",
            Self::VideoExtension => "video_extension",
            Self::LlmPassthrough => "llm_passthrough",
            Self::ContainerCall => "container_call",
        }
    }
}
