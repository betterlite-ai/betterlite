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
use self::handler::{CharacterAnswered, CharacterHandler, CreateHandlerCall, GetHandlerCall};
use self::prepare::{Prepared, prepare_create, prepare_get};
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

// LiteLLM's video_create_character: mock first, then the handler of the form the call asks for
pub fn video_create_character<'call, 'deps, H>(
    handler: &'call mut H,
    deps: CharacterCallDeps<'deps>,
    call: &CreateCharacterCall,
) -> Result<CharacterOutcome<'call>, CharacterCallFailure>
where
    H: CharacterHandler,
    'deps: 'call,
{
    let prepared = prepare_create(handler.logging(), deps.request_timeout_seconds, call)
        .map_err(|error| failure(error, &call.custom_llm_provider, deps.redactor))?;
    let ready = match prepared {
        Prepared::Mock(character) => return Ok(CharacterOutcome::Done(character)),
        Prepared::Call(ready) => ready,
    };
    let provider = call.custom_llm_provider.clone();
    let answer = handler
        .create(CreateHandlerCall {
            name: call.name.clone(),
            video: call.video.clone(),
            provider: call.custom_llm_provider.clone(),
            litellm_params: ready.litellm_params,
            extra_headers: call.extra_headers.clone(),
            timeout: ready.timeout,
            is_async: call.is_async,
        })
        .map_err(|error| failure(CharacterCallError::Handler(error), &provider, deps.redactor))?;
    answered(answer, provider, deps.redactor)
}

// LiteLLM's avideo_create_character: the sync function with the flag set, its future awaited
pub async fn avideo_create_character<H>(
    handler: &mut H,
    deps: CharacterCallDeps<'_>,
    call: &CreateCharacterCall,
) -> Result<CharacterObject, CharacterCallFailure>
where
    H: CharacterHandler,
{
    let mut asynchronous = call.clone();
    asynchronous.is_async = true;
    match video_create_character(handler, deps, &asynchronous)? {
        CharacterOutcome::Done(character) => Ok(character),
        CharacterOutcome::Pending(pending) => pending.await,
    }
}

// LiteLLM's video_get_character: the same steps with the character id as the primary argument
pub fn video_get_character<'call, 'deps, H>(
    handler: &'call mut H,
    deps: CharacterCallDeps<'deps>,
    call: &GetCharacterCall,
) -> Result<CharacterOutcome<'call>, CharacterCallFailure>
where
    H: CharacterHandler,
    'deps: 'call,
{
    let prepared = prepare_get(handler.logging(), deps.request_timeout_seconds, call)
        .map_err(|error| failure(error, &call.custom_llm_provider, deps.redactor))?;
    let ready = match prepared {
        Prepared::Mock(character) => return Ok(CharacterOutcome::Done(character)),
        Prepared::Call(ready) => ready,
    };
    let provider = call.custom_llm_provider.clone();
    let answer = handler
        .get(GetHandlerCall {
            character_id: call.character_id.clone(),
            provider: call.custom_llm_provider.clone(),
            litellm_params: ready.litellm_params,
            extra_headers: call.extra_headers.clone(),
            timeout: ready.timeout,
            is_async: call.is_async,
        })
        .map_err(|error| failure(CharacterCallError::Handler(error), &provider, deps.redactor))?;
    answered(answer, provider, deps.redactor)
}

// LiteLLM's avideo_get_character: the sync function with the flag set, its future awaited
pub async fn avideo_get_character<H>(
    handler: &mut H,
    deps: CharacterCallDeps<'_>,
    call: &GetCharacterCall,
) -> Result<CharacterObject, CharacterCallFailure>
where
    H: CharacterHandler,
{
    let mut asynchronous = call.clone();
    asynchronous.is_async = true;
    match video_get_character(handler, deps, &asynchronous)? {
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
