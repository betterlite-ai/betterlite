pub mod credentials;
pub mod embedding;
pub mod hub;
pub mod huggingface;
pub mod json;
pub mod predibase;
pub mod replicate;
pub mod rerank;
mod tests;
pub mod tokens;
pub mod transport;
use self::credentials::Globals;
use crate::oci_common::Clock;
use crate::operation::OperationError;
use crate::predibase::error::{PredibaseError, PredibaseSetupError};
use crate::predibase::reply::TokenCounter as PredibaseTokenCounter;
use crate::prepared::PreparedRequest;
use crate::replicate::error::ReplicateFailure;
use crate::replicate::poll::transport::ReplicateTransport;
use crate::replicate::poll::{ReplicatePolling, ReplicateSleeper};
use crate::replicate::reply::PostCallLogger;
use crate::replicate::reply::TokenCounter as ReplicateTokenCounter;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_context::chat_config_selection::tags::{ChatConfigTag, ConfigSelection};
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::huggingface_chat::error::HuggingfaceChatError;
use betterlite_openai::huggingface_embedding::error::HuggingfaceEmbeddingError;
use betterlite_openai::huggingface_embedding::response::OrdinaryEncoder;
use betterlite_openai::huggingface_rerank::error::HuggingfaceRerankError;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::custom::CustomPromptTemplate;
use betterlite_tokenizer::engine::error::EngineError;
use chrono::{DateTime, Utc};
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::future::Future;
use std::str::Utf8Error;
use std::sync::Arc;
use std::time::Duration;
pub type CustomPromptTable = BTreeMap<String, CustomPromptTemplate>;
#[derive(Debug, thiserror::Error)]
pub enum HubDispatchError {
    #[error(transparent)]
    Chat(#[from] HuggingfaceChatError),
    #[error(transparent)]
    Embedding(#[from] HuggingfaceEmbeddingError),
    #[error(transparent)]
    Rerank(#[from] HuggingfaceRerankError),
    #[error(transparent)]
    Replicate(#[from] ReplicateFailure),
    #[error(transparent)]
    PredibaseSetup(#[from] PredibaseSetupError),
    #[error(transparent)]
    Predibase(#[from] PredibaseError),
    #[error(transparent)]
    Operation(#[from] OperationError),
    #[error("the {field} value must be a string")]
    FieldType { field: &'static str },
    #[error("the {url} endpoint is not a valid URL")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the request headers cannot be carried")]
    Headers {
        #[source]
        source: http::Error,
    },
    #[error("the request body cannot be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the request body is not a JSON object")]
    BodyNotObject,
    #[error("the reply body is not UTF-8")]
    BodyNotUtf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("the pre_call logging hook failed")]
    PreCall {
        #[source]
        source: PrePostError,
    },
    #[error("the Replicate polling settings cannot be read")]
    Environment {
        #[source]
        source: betterlite_env::EnvError,
    },
    #[error("the bundled cl100k_base vocabulary is missing")]
    MissingVocabulary,
    #[error("the cl100k_base tokenizer cannot be built")]
    Encoding {
        #[source]
        source: EngineError,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubProvider {
    HuggingFace,
    Replicate,
    Predibase,
}
#[must_use]
pub const fn select(tag: ChatConfigTag) -> Option<HubProvider> {
    if matches!(tag, ChatConfigTag::HuggingFaceChat) {
        return Some(HubProvider::HuggingFace);
    }
    if matches!(tag, ChatConfigTag::Replicate) {
        return Some(HubProvider::Replicate);
    }
    if matches!(tag, ChatConfigTag::Predibase) {
        return Some(HubProvider::Predibase);
    }
    None
}
pub struct ChatContext<'source> {
    pub model: &'source str,
    pub messages: &'source mut Vec<Message>,
    pub optional_params: &'source mut Object,
    pub api_key: Option<&'source SecretString>,
    pub api_base: Option<&'source str>,
    pub headers: Option<&'source Headers>,
    pub custom_prompt_dict: Option<&'source CustomPromptTable>,
    pub acompletion: bool,
}
pub struct ReplicateServices<'deps> {
    pub transport: Arc<dyn ReplicateTransport>,
    pub sleeper: Arc<dyn ReplicateSleeper>,
    pub logger: &'deps dyn PostCallLogger,
}
pub struct DispatchDeps<'deps> {
    pub globals: &'deps Globals<'deps>,
    pub environment: &'deps (dyn EnvironmentLookup + Sync),
    pub hub: &'deps dyn transport::HubGetAsync,
    pub post: &'deps dyn transport::PostTransport,
    pub encoder: &'deps (dyn OrdinaryEncoder + Sync),
    pub replicate: ReplicateServices<'deps>,
    pub prompts: &'deps PromptFactory,
    pub replicate_counter: &'deps dyn ReplicateTokenCounter,
    pub predibase_counter: &'deps dyn PredibaseTokenCounter,
    pub clock: &'deps dyn Clock,
    pub default_max_tokens: i64,
    pub now: DateTime<Utc>,
    pub timeout: Duration,
}
pub enum HubCompletion {
    HuggingFace(Box<PreparedRequest>),
    Replicate(Box<replicate::Completion>),
    Predibase(Box<predibase::Completion>),
}
pub fn polling(env: &dyn EnvironmentLookup) -> Result<ReplicatePolling, HubDispatchError> {
    ReplicatePolling::from_env_values(
        env.lookup("DEFAULT_REPLICATE_POLLING_RETRIES").as_deref(),
        env.lookup("DEFAULT_REPLICATE_POLLING_DELAY_SECONDS")
            .as_deref(),
        env.lookup("REPLICATE_POLLING_DELAY_SECONDS").as_deref(),
        env.lookup("REPLICATE_MODEL_NAME_WITH_ID_LENGTH").as_deref(),
    )
    .map_err(|source| HubDispatchError::Environment { source })
}
pub fn dispatch<'call>(
    selection: &ConfigSelection<ChatConfigTag>,
    call: &'call mut ChatContext<'_>,
    deps: &'call DispatchDeps<'_>,
    log: &'call mut LogContext<'_, '_>,
) -> impl Future<Output = Result<Option<HubCompletion>, HubDispatchError>> + Send + 'call {
    let provider = select(selection.tag);
    async move {
        match provider {
            None => Ok(None),
            Some(HubProvider::HuggingFace) => {
                let body = huggingface::request_body(call, deps).await?;
                let request = huggingface::chat_request(call, deps, &body)?;
                Ok(Some(HubCompletion::HuggingFace(Box::new(request))))
            }
            Some(HubProvider::Replicate) => Ok(Some(HubCompletion::Replicate(Box::new(
                replicate::complete(call, deps, log).await?,
            )))),
            Some(HubProvider::Predibase) => Ok(Some(HubCompletion::Predibase(Box::new(
                predibase::complete(call, deps, log).await?,
            )))),
        }
    }
}
