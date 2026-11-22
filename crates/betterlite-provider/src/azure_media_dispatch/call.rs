use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_azure::credentials::source::CallableTokenProvider;
use betterlite_azure::endpoints::projection::{
    CallableProvider, LiteLlmParams, SessionKind, SuppliedClient,
};
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::image_edits::file::ImageEditFile;
use indexmap::IndexMap;
use secrecy::SecretString;
use uuid::Uuid;

// The typed call the composition executes; operation.rs carries the unit marker above it and the caller keeps the call beside it.
pub enum AzureMediaOperation {
    ImageGeneration(Box<ImageGenerationOperation>),
    ImageEdit(Box<ImageEditOperation>),
    OpenAiSpeech,
    NativeSsml,
    Transcription(Box<TranscriptionOperation>),
    NativeTranscription(Box<NativeTranscriptionOperation>),
}

// AzureChatCompletion.image_generation's arguments as the caller resolved them
#[derive(Default)]
pub struct ImageGenerationOperation {
    pub model: Option<String>,
    pub prompt: String,
    pub timeout: Option<f64>,
    pub optional_params: IndexMap<String, Json>,
    pub headers: Vec<(String, String)>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub azure_ad_token: Option<SecretString>,
    pub provider: Option<Arc<dyn CallableTokenProvider>>,
    pub base_model: Option<String>,
    pub params: LiteLlmParams,
    pub delegate_async: bool,
    pub now_seconds: i64,
}

// AzureImageEditConfig's inputs: the edit form fields, the auth values and the caller's boundary
#[derive(Default)]
pub struct ImageEditOperation {
    pub model: String,
    pub prompt: Option<String>,
    pub image: Option<Vec<Option<ImageEditFile>>>,
    pub mask: Option<Vec<Option<ImageEditFile>>>,
    pub options: IndexMap<String, Json>,
    // the optional request params' extra headers, then the call's own extra_headers applied after the auth headers
    pub headers: BTreeMap<String, String>,
    pub extra_headers: BTreeMap<String, String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub provider: Option<Arc<dyn CallableTokenProvider>>,
    pub params: LiteLlmParams,
    pub boundary: Uuid,
    pub now_seconds: i64,
}

// AzureAudioTranscription.audio_transcriptions takes the processed file by reference and the rest of the call by value.
pub struct TranscriptionOperation {
    pub input: String,
    pub file: ProcessedAudioFile,
    pub boundary: Uuid,
    pub model: String,
    pub optional_params: IndexMap<String, Json>,
    pub atranscription: bool,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub azure_ad_token: Option<SecretString>,
    pub max_retries: Option<i64>,
    pub timeout: Duration,
    pub params: LiteLlmParams,
    pub provider: Option<CallableProvider>,
    pub supplied: Option<SuppliedClient>,
    pub session: SessionKind,
    pub refresh_enabled: bool,
}

// AzureSpeechAudioTranscriptionConfig's inputs: the audio, the optional params and the already resolved environment
pub struct NativeTranscriptionOperation {
    pub filename: String,
    pub content: Vec<u8>,
    pub optional_params: IndexMap<String, Json>,
    pub headers: BTreeMap<String, String>,
    pub api_key: Option<SecretString>,
    pub environment_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub environment_base: Option<String>,
    pub timeout: Duration,
}
