pub mod error;
pub mod handler;
mod logging;
mod mock;
mod params;
mod prepare;
mod tests;

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use self::error::{CharacterCallError, CharacterCallFailure, failure};
use self::handler::{CharacterAnswered, CharacterHandler};
use self::prepare::{CharacterCall, Prepared};
use crate::api::bfl::edit::media::SuppliedMedia;
use crate::api::video_records::CharacterObject;

// LiteLLM declares the provider and the timeout argument with these defaults
pub const DEFAULT_PROVIDER: &str = "openai";
pub const DEFAULT_TIMEOUT_SECONDS: f64 = 600.0;
pub(crate) const CREATE_CALL_TYPE: &str = "video_create_character";
pub(crate) const GET_CALL_TYPE: &str = "video_get_character";

// every keyword video_create_character takes, typed the way the caller passes it
#[derive(Clone, Debug)]
pub struct CreateCharacterCall {
    pub name: String,
    // FileTypes: bytes, a file, a path or URL string, or a list of them
    pub video: SuppliedMedia,
    // a zero is falsey in LiteLLM's `timeout or DEFAULT_REQUEST_TIMEOUT`, so it falls back too
    pub timeout: Option<f64>,
    pub custom_llm_provider: String,
    pub extra_headers: Option<BTreeMap<String, String>>,
    // accepted and never sent on a character call
    pub extra_query: Option<IndexMap<String, Json>>,
    pub extra_body: Option<IndexMap<String, Json>>,
    pub mock_response: Option<Json>,
    // from kwargs, the value update_environment_variables logs beside the primary argument
    pub litellm_call_id: Option<String>,
    // every other keyword, which reads as litellm_params and the logging user
    pub extra: IndexMap<String, Json>,
    // the async form sets this; LiteLLM reads it from kwargs.pop("async_call")
    pub is_async: bool,
}

// every keyword video_get_character takes
#[derive(Clone, Debug)]
pub struct GetCharacterCall {
    pub character_id: String,
    pub timeout: Option<f64>,
    pub custom_llm_provider: String,
    pub extra_headers: Option<BTreeMap<String, String>>,
    pub extra_query: Option<IndexMap<String, Json>>,
    pub extra_body: Option<IndexMap<String, Json>>,
    pub mock_response: Option<Json>,
    pub litellm_call_id: Option<String>,
    pub extra: IndexMap<String, Json>,
    pub is_async: bool,
}

// both signatures declare the same defaults: the openai provider and 600 seconds
macro_rules! character_call_default {
    ($call:ident { $($field:ident: $value:expr),* $(,)? }) => {
        impl Default for $call {
            fn default() -> Self {
                Self {
                    $($field: $value,)*
                }
            }
        }
    };
}

character_call_default!(CreateCharacterCall {
    name: String::new(),
    video: SuppliedMedia::Text(String::new()),
    timeout: Some(DEFAULT_TIMEOUT_SECONDS),
    custom_llm_provider: DEFAULT_PROVIDER.to_owned(),
    extra_headers: None,
    extra_query: None,
    extra_body: None,
    mock_response: None,
    litellm_call_id: None,
    extra: IndexMap::new(),
    is_async: false,
});

character_call_default!(GetCharacterCall {
    character_id: String::new(),
    timeout: Some(DEFAULT_TIMEOUT_SECONDS),
    custom_llm_provider: DEFAULT_PROVIDER.to_owned(),
    extra_headers: None,
    extra_query: None,
    extra_body: None,
    mock_response: None,
    litellm_call_id: None,
    extra: IndexMap::new(),
    is_async: false,
});

// the collaborators the operations need: B185's redactor and B198's REQUEST_TIMEOUT seconds
#[derive(Clone, Copy)]
pub struct CharacterCallDeps<'deps> {
    pub redactor: &'deps Redactor,
    // LiteLLM's request_timeout constant that REQUEST_TIMEOUT overrides, reached only where LiteLLM reaches it
    pub request_timeout_seconds: f64,
}

// the finished character, or the future a handler raised for the async form
pub enum CharacterOutcome<'call> {
    Done(CharacterObject),
    Pending(
        Pin<Box<dyn Future<Output = Result<CharacterObject, CharacterCallFailure>> + Send + 'call>>,
    ),
}

impl fmt::Debug for CharacterOutcome<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Done(character) => formatter.debug_tuple("Done").field(character).finish(),
            Self::Pending(_) => formatter.write_str("Pending(<future>)"),
        }
    }
}

// ast-grep-ignore: no-one-line-functions - LiteLLM's video_create_character: mock first, then one call
pub fn video_create_character<'call>(
    handler: &'call mut dyn CharacterHandler,
    deps: CharacterCallDeps<'call>,
    call: &CreateCharacterCall,
) -> Result<CharacterOutcome<'call>, CharacterCallFailure> {
    character_call(handler, deps, CharacterCall::Create(call), false)
}

// ast-grep-ignore: no-one-line-functions - LiteLLM's avideo_create_character: the whole flow is awaited
pub async fn avideo_create_character<'call>(
    handler: &'call mut dyn CharacterHandler,
    deps: CharacterCallDeps<'call>,
    call: &CreateCharacterCall,
) -> Result<CharacterObject, CharacterCallFailure> {
    awaited(handler, deps, CharacterCall::Create(call), true).await
}

// ast-grep-ignore: no-one-line-functions - LiteLLM's video_get_character: mock first, then one call
pub fn video_get_character<'call>(
    handler: &'call mut dyn CharacterHandler,
    deps: CharacterCallDeps<'call>,
    call: &GetCharacterCall,
) -> Result<CharacterOutcome<'call>, CharacterCallFailure> {
    character_call(handler, deps, CharacterCall::Get(call), false)
}

// ast-grep-ignore: no-one-line-functions - LiteLLM's avideo_get_character: the whole flow is awaited
pub async fn avideo_get_character<'call>(
    handler: &'call mut dyn CharacterHandler,
    deps: CharacterCallDeps<'call>,
    call: &GetCharacterCall,
) -> Result<CharacterObject, CharacterCallFailure> {
    awaited(handler, deps, CharacterCall::Get(call), true).await
}

// the steps both operations run, shared so the two entry points only name their call
fn character_call<'call>(
    handler: &'call mut dyn CharacterHandler,
    deps: CharacterCallDeps<'call>,
    call: CharacterCall<'_>,
    asynchronous: bool,
) -> Result<CharacterOutcome<'call>, CharacterCallFailure> {
    let prepared = call
        .prepare(handler.logging(), deps.request_timeout_seconds)
        .map_err(|error| failure(error, call.provider(), deps.redactor))?;
    let ready = match prepared {
        Prepared::Mock(character) => return Ok(CharacterOutcome::Done(character)),
        Prepared::Call(ready) => ready,
    };
    let provider = call.provider().to_owned();
    let answer = handler
        .handle(call.handler_call(ready, provider.clone(), asynchronous))
        .map_err(|error| failure(CharacterCallError::Handler(error), &provider, deps.redactor))?;
    answered(answer, provider, deps.redactor)
}

// the async entry resolves whatever form the handler answered in
async fn awaited<'call>(
    handler: &'call mut dyn CharacterHandler,
    deps: CharacterCallDeps<'call>,
    call: CharacterCall<'_>,
    asynchronous: bool,
) -> Result<CharacterObject, CharacterCallFailure> {
    match character_call(handler, deps, call, asynchronous)? {
        CharacterOutcome::Done(character) => Ok(character),
        CharacterOutcome::Pending(pending) => pending.await,
    }
}

// a handler's answer becomes the caller's outcome, a pending one carrying the mapped failure
fn answered<'call>(
    answer: CharacterAnswered<'call>,
    provider: String,
    redactor: &'call Redactor,
) -> Result<CharacterOutcome<'call>, CharacterCallFailure> {
    match answer {
        CharacterAnswered::Done(character) => Ok(CharacterOutcome::Done(character)),
        CharacterAnswered::Pending(pending) => {
            Ok(CharacterOutcome::Pending(Box::pin(async move {
                pending.await.map_err(|error| {
                    failure(CharacterCallError::Handler(error), &provider, redactor)
                })
            })))
        }
    }
}
