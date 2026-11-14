use betterlite_openai::api::audio_file_input::AudioFileError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DeepgramTranscriptionError {
    #[error("the audio file could not be processed")]
    Audio {
        #[from]
        source: AudioFileError,
    },
    // LiteLLM wraps every decode failure with the raw body text in a ValueError
    #[error("Error transforming Deepgram response: {source}\nResponse: {body}")]
    Transform {
        body: String,
        #[source]
        source: Box<Self>,
    },
    #[error("the reply is larger than {limit} bytes")]
    ReplyTooLarge { limit: usize },
    #[error("the reply does not have the Deepgram shape")]
    Reply {
        #[from]
        source: serde_json::Error,
    },
    #[error("results.channels is empty")]
    NoChannel,
    #[error("the first channel has no alternatives")]
    NoAlternative,
    #[error("the first alternative has no transcript")]
    NoTranscript,
    #[error("the first alternative has null paragraphs")]
    NullParagraphs,
    #[error("paragraphs has no transcript")]
    NoParagraphTranscript,
    #[error("the first alternative has null words")]
    NullWords,
    #[error("a diarized word has a null punctuated_word")]
    NullPunctuatedWord,
}
