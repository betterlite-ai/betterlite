use betterlite_streaming::stream_content_assembly::AssemblyError;
use serde_json::Error as JsonError;
use thiserror::Error;

use crate::model_urls::error::VertexUrlError;
use crate::token_cache::error::CredentialCacheError;

#[derive(Debug, Error)]
pub enum VertexSpeechError {
    #[error("the Vertex AI access token could not be minted")]
    Mint(#[source] CredentialCacheError),
    #[error("the vertex location is not valid")]
    Location(#[source] VertexUrlError),
    #[error("the Vertex AI TTS response is {size} bytes, over the {limit} byte limit")]
    ResponseTooLarge { size: usize, limit: usize },
    #[error("the Vertex AI TTS response is not valid JSON")]
    ResponseJson(#[source] JsonError),
    #[error("the Vertex AI TTS response is not a JSON object")]
    ResponseNotObject,
    #[error("the Vertex AI TTS response carries no audioContent")]
    MissingAudioContent,
    #[error("the audioContent of the Vertex AI TTS response is not a string")]
    AudioContentNotText,
    #[error("the audioContent of the Vertex AI TTS response cannot be decoded")]
    Audio(#[source] AssemblyError),
    #[error("SSML input is required when use_ssml is set")]
    SsmlRequired,
    #[error("either a text or an ssml input is required")]
    InputRequired,
    #[error("only one of text and ssml may be provided")]
    BothInputs,
    #[error("the vertex voice record is not a valid voice")]
    VoiceRecord(#[source] JsonError),
}
