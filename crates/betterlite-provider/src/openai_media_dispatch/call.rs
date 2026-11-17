use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::image_edits::request::ImageEditRequest;
use betterlite_openai::image_generation::call::ImageGenerationCall;
use betterlite_openai::image_variations::ImageVariationRequest;
use betterlite_openai::speech::client::SpeechConnection;
use betterlite_openai::speech::request::SpeechOptions;
use betterlite_openai::transcription::client::TranscriptionConnection;
use betterlite_openai::transcription::config::TranscriptionFamily;
use betterlite_openai::videos::VideoOptions;
use betterlite_openai::videos::create::VideoCreate;
use indexmap::IndexMap;
use uuid::Uuid;

// The typed call the composition executes; operation.rs carries the unit marker above it and the caller keeps the call beside it.
pub enum OpenAiMediaOperation {
    ImageGeneration(Box<ImageGenerationCall>),
    ImageEdit(Box<ImageEditCall>),
    ImageVariation(Box<ImageVariationRequest>),
    Speech(Box<SpeechOperation>),
    Transcription(Box<TranscriptionOperation>),
    Video(Box<VideoOperation>),
    VideoEdit(Box<VideoEditOperation>),
}

// The edit takes the api_base the caller resolved; B512 appends its own edits path, and B002 owns the default.
pub struct ImageEditCall {
    pub api_base: String,
    pub request: ImageEditRequest,
}

// audio_speech reads the request fields by reference, so the owned call hands them out here.
pub struct SpeechOperation {
    pub model: String,
    pub input: String,
    pub voice: String,
    pub options: SpeechOptions,
    pub connection: SpeechConnection,
    pub aspeech: bool,
}

// audio_transcriptions takes the processed file by reference and the rest of the call by value.
pub struct TranscriptionOperation {
    pub input: String,
    pub file: ProcessedAudioFile,
    pub connection: TranscriptionConnection,
    pub boundary: Uuid,
    pub model: String,
    pub family: Option<TranscriptionFamily>,
    pub optional_params: IndexMap<String, Json>,
    pub atranscription: bool,
}

// get_complete_url owns the video base default; provider is LiteLLM's custom_llm_provider for the reply wrap.
pub struct VideoOperation {
    pub api_base: Option<String>,
    pub provider: Option<String>,
    pub call: VideoCall,
}

pub enum VideoCall {
    Create(Box<VideoCreateCall>),
    List(Box<VideoListCall>),
    Status(Box<VideoStatusCall>),
    Delete(Box<VideoDeleteCall>),
    Content(Box<VideoContentCall>),
    Remix(Box<VideoRemixCall>),
}

// The reply field carries the upstream body the leaf's response transform decodes.
pub struct VideoCreateCall {
    pub model: String,
    pub prompt: String,
    pub options: VideoCreate,
    pub input_reference: Option<Vec<u8>>,
    pub reply: Vec<u8>,
}

pub struct VideoListCall {
    pub after: Option<String>,
    pub limit: Option<i64>,
    pub order: Option<String>,
    pub extra_query: VideoOptions,
    pub reply: Vec<u8>,
}

pub struct VideoStatusCall {
    pub video_id: String,
    pub reply: Vec<u8>,
}

pub struct VideoDeleteCall {
    pub video_id: String,
    pub reply: Vec<u8>,
}

pub struct VideoContentCall {
    pub video_id: String,
    pub variant: Option<String>,
    pub reply: Vec<u8>,
}

pub struct VideoRemixCall {
    pub video_id: String,
    pub prompt: String,
    pub extra_body: VideoOptions,
    pub reply: Vec<u8>,
}

pub struct VideoEditOperation {
    pub api_base: Option<String>,
    pub provider: Option<String>,
    pub call: VideoEditCall,
}

pub enum VideoEditCall {
    Edit(Box<VideoEditInput>),
    Extension(Box<VideoExtensionInput>),
    CreateCharacter(Box<CharacterCreateInput>),
    GetCharacter(Box<CharacterGetInput>),
}

pub struct VideoEditInput {
    pub prompt: String,
    pub video_id: String,
    pub video_file: Option<Vec<u8>>,
    pub extra_body: Option<IndexMap<String, Json>>,
    pub reply: Vec<u8>,
}

pub struct VideoExtensionInput {
    pub prompt: String,
    pub video_id: String,
    pub seconds: String,
    pub extra_body: Option<IndexMap<String, Json>>,
    pub reply: Vec<u8>,
}

pub struct CharacterCreateInput {
    pub name: String,
    pub video: Vec<u8>,
    pub reply: Vec<u8>,
}

pub struct CharacterGetInput {
    pub character_id: String,
    pub reply: Vec<u8>,
}
