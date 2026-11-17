use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_models::catalog::ModelCatalog;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::factory::custom::CustomPromptTemplate;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::message_content::Message;
use betterlite_tokenizer::count::Counter;
use jiff::Timestamp;
use secrecy::SecretString;
use uuid::Uuid;

use crate::cache_lookup::transport::CacheTransport;
use crate::gemini_http::call::{Clients, VertexLlm};
use crate::gemini_http::stream::GeminiHttpStream;
use crate::gemini_media::gcs_metadata::GcsTransport;
use crate::gemini_response::complete::CompleteResponse;
use crate::gemini_response_parts::projection::CallMode;
use crate::model_policy::capabilities::{ModelCapabilityLookup, VertexCapabilityProvider};

use self::error::GeminiCallError;

mod async_call;
mod body;
pub mod error;
pub mod selection;
mod sync_call;
mod target;

mod tests;

// LiteLLM's completion arguments: the request, the caller's provider settings and the two routing flags
pub struct GeminiCall<'call> {
    pub provider: VertexCapabilityProvider,
    pub model: &'call str,
    pub messages: &'call [Message],
    pub optional_params: &'call Object,
    pub litellm_params: &'call Object,
    pub api_base: Option<&'call str>,
    pub gemini_api_key: Option<&'call SecretString>,
    pub project: Option<&'call str>,
    pub location: Option<&'call str>,
    pub credentials: Option<&'call ConfiguredCredentials>,
    pub extra_headers: &'call BTreeMap<String, String>,
    pub timeout: Option<Duration>,
    pub acompletion: bool,
    pub modify_params: bool,
    pub now: Timestamp,
    pub call_mode: CallMode,
}

// Everything the two branches read from the environment, passed in so neither is built inside a call
pub struct CallDeps<'deps, Cache, Fetch, Log, Console> {
    pub catalog: &'deps ModelCatalog,
    pub llm: &'deps VertexLlm,
    pub clients: &'deps Clients,
    pub capabilities: &'deps (dyn ModelCapabilityLookup + Sync),
    pub cache: &'deps Cache,
    pub fetcher: &'deps Fetch,
    pub gcs: &'deps dyn GcsTransport,
    pub counter: &'deps Counter,
    pub hooks: &'deps VerboseHooks<Log, Console>,
    pub schema_template: Option<&'deps CustomPromptTemplate>,
}

// Boxed to stay within 3x of each other, the limit variant_size_differences enforces
pub enum GeminiReply {
    Complete(Box<CompleteResponse>),
    Stream(Box<GeminiHttpStream>),
}

// LiteLLM pops stream, then routes on acompletion and the literal true check
pub async fn completion<Cache, Fetch, Log, Console>(
    call: &GeminiCall<'_>,
    deps: &CallDeps<'_, Cache, Fetch, Log, Console>,
    identifiers: Box<dyn FnMut() -> Uuid + Send>,
) -> Result<GeminiReply, GeminiCallError>
where
    Cache: CacheTransport,
    Fetch: RemoteMediaFetcher + Sync,
    Log: LoggerSink + Sync,
    Console: ConsoleSink + Sync,
{
    let mut optional_params = call.optional_params.clone();
    let stream = selection::pop_stream(&mut optional_params)?;
    let selected = selection::select(call.acompletion, stream);
    if selected.asynchronous {
        async_call::completion(
            call,
            &optional_params,
            deps,
            selected.streaming,
            identifiers,
        )
        .await
    } else {
        sync_call::completion(
            call,
            &optional_params,
            deps,
            selected.streaming,
            identifiers,
        )
        .await
    }
}
