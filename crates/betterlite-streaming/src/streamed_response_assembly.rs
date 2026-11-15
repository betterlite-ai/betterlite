use betterlite_error::error::details::ApiDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::secret::Redactor;
use betterlite_redact::structured::Payload;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use thiserror::Error;

use crate::stream_base_assembly::StreamedChunks;
use crate::stream_base_assembly::provider_metadata::{
    ChatConfigSelection, ProviderMetadataProjection, ProviderResolution,
};
use crate::stream_base_assembly::response::build_base_response;
use crate::stream_content_assembly::AssemblyError as ContentAssemblyError;
use crate::stream_tool_assembly::AssemblyError as ToolAssemblyError;
use crate::stream_usage_assembly::{UsageAssemblyError, UsageTokenCounter};

pub mod records;
pub mod response;

mod complex;
mod content;
mod cost;
mod finish;
mod project;
mod simple;
mod tests;
mod text;

pub use cost::{CostError, CostInput, StreamCostCalculator, StreamLogging};
pub use records::Chunk;
pub use response::{ChatResponse, Response, TextResponse};

use finish::ReasoningMode;
use response::{Draft, Extras};

const SOURCE_MESSAGE: &str = "Error building chunks for logging/streaming usage calculation";
const ERROR_STATUS: u16 = 500;

#[derive(Debug, Error)]
pub enum AssemblyError {
    #[error("chunks is required")]
    MissingChunks,
    #[error("the last chunk has no first choice")]
    MissingLastChoice,
    #[error("the base response has no choice")]
    MissingChoice,
    #[error("the tool assembly returned a function output")]
    WrongToolOutput,
    #[error("the function assembly returned a tool output")]
    WrongFunctionOutput,
    #[error("provider_specific_fields is not a JSON object")]
    InvalidProviderFields,
    #[error(transparent)]
    Base(#[from] crate::stream_base_assembly::StreamBaseAssemblyError),
    #[error(transparent)]
    Tool(#[from] ToolAssemblyError),
    #[error(transparent)]
    Content(#[from] ContentAssemblyError),
    #[error(transparent)]
    Usage(#[from] UsageAssemblyError),
    #[error("the token count failed: {source}")]
    TokenCount {
        #[from]
        source: CountError,
    },
    #[error("the token count overflowed")]
    Overflow,
    #[error("projecting the chunks failed: {source}")]
    Projection {
        #[from]
        source: serde_json::Error,
    },
    #[error("the cost calculation failed: {source}")]
    Cost {
        #[from]
        source: CostError,
    },
}

impl AssemblyError {
    // Every failure inside the builder becomes litellm.APIError(status_code=500, llm_provider="", model="").
    #[must_use]
    pub fn classify(&self) -> ErrorClass {
        ErrorClass::Api(Box::new(ApiDetails {
            context: ErrorContext {
                message: SOURCE_MESSAGE.to_owned(),
                model: Some(String::new()),
                provider: Some(String::new()),
                max_retries: None,
                num_retries: None,
                body_type: None,
                body_param: None,
                body_code: None,
            },
            status: ERROR_STATUS,
        }))
    }
}

// B2232's provider metadata seams, which the assembler wires to the logging provider.
pub struct MetadataServices<'services> {
    pub redactor: &'services Redactor,
    pub resolver: &'services dyn ProviderResolution,
    pub selector: &'services dyn ChatConfigSelection,
    pub projection: Option<&'services dyn ProviderMetadataProjection>,
}

pub struct Request<'request> {
    pub chunks: Option<&'request [Chunk]>,
    pub messages: Option<&'request [Message]>,
    pub count_prompt_tokens: Option<&'request (dyn Fn() -> Result<i64, CountError> + Send + Sync)>,
    pub logging: Option<&'request dyn StreamLogging>,
    pub costs: &'request dyn StreamCostCalculator,
    pub counter: &'request dyn UsageTokenCounter,
    pub metadata: Option<&'request MetadataServices<'request>>,
    // start_time and end_time are accepted by LiteLLM and never read; the caller's clock fills time defaults.
    pub now: i64,
}

pub async fn stream_chunk_builder<L, C>(
    request: &Request<'_>,
    hooks: &VerboseHooks<L, C>,
) -> Result<Option<Response>, AssemblyError>
where
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    match build(request, hooks).await {
        Ok(response) => Ok(response),
        Err(error) => {
            let _logged = hooks.utils_print_verbose(
                Payload::String(format!(
                    "litellm.main.py::stream_chunk_builder() - Exception occurred - {error}"
                )),
                "ERROR",
                &Payload::Bool(true),
            );
            Err(error)
        }
    }
}

async fn build<L, C>(
    request: &Request<'_>,
    hooks: &VerboseHooks<L, C>,
) -> Result<Option<Response>, AssemblyError>
where
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    let Some(chunks) = request.chunks else {
        return Err(AssemblyError::MissingChunks);
    };
    if chunks.is_empty() {
        return Ok(None);
    }
    let streamed = StreamedChunks::new(chunks.to_vec())?;
    let first_choice = streamed
        .chunks
        .iter()
        .find(|chunk| !chunk.choices.is_empty())
        .and_then(|chunk| chunk.choices.first());
    if first_choice.is_some_and(|choice| matches!(choice.text, records::TextField::Present(_))) {
        let assembled = text::assemble_text(request, &streamed.chunks, hooks).await?;
        return Ok(Some(Response::Text(assembled)));
    }
    let chat = assemble_chat(request, &streamed, hooks).await?;
    Ok(Some(Response::Chat(Box::new(chat))))
}

async fn assemble_chat<L, C>(
    request: &Request<'_>,
    streamed: &StreamedChunks,
    hooks: &VerboseHooks<L, C>,
) -> Result<ChatResponse, AssemblyError>
where
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    let mut base = build_base_response(streamed)?;
    let simple = simple::apply(&mut base, &streamed.chunks);
    let mut draft = Draft {
        base,
        extras: Extras::default(),
    };
    let mode = if simple {
        ReasoningMode::Zero
    } else {
        complex::apply(&mut draft, &streamed.chunks, request.now)?;
        ReasoningMode::Counted
    };
    finish::finish_chat(request, streamed, draft, mode, hooks).await
}
