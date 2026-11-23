use betterlite_config::loader::ConfigError;
use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::fetch::FetchError;
use betterlite_tokenizer::engine::hub::HubError;
use betterlite_tokenizer::engine::selection::SelectionError;
use betterlite_tokenizer::image::error::ImageError;
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyValueError};

create_exception!(betterlite_native, NativeTokensError, PyException);

#[derive(Debug, thiserror::Error)]
pub(crate) enum TokensError {
    #[error("the {field} could not be read as JSON of the expected shape: {source}")]
    Json {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("text and messages cannot both be set")]
    TextAndMessages,
    // LiteLLM capitalizes the first word of this one message and keeps the rest lowercase
    #[error("Either text or messages must be provided")]
    MissingInput,
    #[error("tools or tool_choice cannot be set if using text")]
    ToolsWithText,
    #[error(
        "a hugging face tokenizer always drops special tokens, so skip_special_tokens=False cannot be honoured"
    )]
    SkipSpecialTokens,
    #[error(transparent)]
    Count(CountError),
    #[error(transparent)]
    Hub(HubError),
    #[error(transparent)]
    Fetch(FetchError),
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error(transparent)]
    Image(#[from] ImageError),
    #[error("the tokenizer handle {tokenizer} is not loaded")]
    UnknownTokenizer { tokenizer: u64 },
    #[error("the tokenizer handle store is locked by a failed call")]
    StoreLocked,
    #[error("the configuration could not be read: {source}")]
    Config {
        #[from]
        source: ConfigError,
    },
    #[error("the proxy environment could not be read: {source}")]
    Proxy {
        #[from]
        source: EnvError,
    },
    #[error("the outbound http client could not be built: {source}")]
    Client {
        #[from]
        source: HttpError,
    },
}

impl From<CountError> for TokensError {
    fn from(error: CountError) -> Self {
        match error {
            // the counter's own texts differ from LiteLLM's token_counter, which the three below keep
            CountError::TextAndMessages => Self::TextAndMessages,
            CountError::MissingInput => Self::MissingInput,
            CountError::ToolsWithText => Self::ToolsWithText,
            other @ (CountError::UnsupportedContent { .. }
            | CountError::ContentField { .. }
            | CountError::Selection(_)
            | CountError::Fetch(_)
            | CountError::Engine(_)
            | CountError::Tools(_)
            | CountError::Json { .. }
            | CountError::Overflow
            | CountError::Boundary
            | CountError::LimitsOverflow) => Self::Count(other),
        }
    }
}

// LiteLLM's huggingface_hub errors name the repository, and the selector's own text does not, so the cause keeps its own message
impl From<SelectionError> for TokensError {
    fn from(error: SelectionError) -> Self {
        match error {
            SelectionError::Hub(hub) => Self::Hub(hub),
            SelectionError::Fetch(fetch) => Self::Fetch(fetch),
            SelectionError::Engine(engine) => Self::Engine(engine),
        }
    }
}

impl From<TokensError> for PyErr {
    fn from(error: TokensError) -> Self {
        match &error {
            // LiteLLM raises ValueError for bad arguments and for a content block it cannot count
            TokensError::Json { .. }
            | TokensError::TextAndMessages
            | TokensError::MissingInput
            | TokensError::ToolsWithText
            | TokensError::UnknownTokenizer { .. }
            | TokensError::Count(
                CountError::UnsupportedContent { .. } | CountError::ContentField { .. },
            ) => PyValueError::new_err(error.to_string()),
            TokensError::Count(_)
            | TokensError::SkipSpecialTokens
            | TokensError::Hub(_)
            | TokensError::Fetch(_)
            | TokensError::Engine(_)
            | TokensError::Image(_)
            | TokensError::StoreLocked
            | TokensError::Config { .. }
            | TokensError::Proxy { .. }
            | TokensError::Client { .. } => NativeTokensError::new_err(error.to_string()),
        }
    }
}

mod tests;
