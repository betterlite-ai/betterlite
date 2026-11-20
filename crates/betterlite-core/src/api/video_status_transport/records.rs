use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::error::VideoStatusError;
use crate::api::video_records::VideoObject;

pub type Params = IndexMap<String, Json>;
pub type Headers = IndexMap<String, String>;

// The arguments of validate_environment: the caller's headers (or an empty map), model="", the key and the params
#[derive(Debug, Clone, Copy)]
pub struct VideoEnvironment<'request> {
    pub headers: &'request Headers,
    pub model: &'request str,
    pub api_key: Option<&'request SecretString>,
    pub litellm_params: Option<&'request Params>,
}

// The last three arguments of every video request transform
#[derive(Debug, Clone, Copy)]
pub struct VideoRequestContext<'request> {
    pub api_base: &'request str,
    pub litellm_params: &'request Params,
    pub headers: &'request Headers,
}

// What a response transform gets back: the raw HTTP answer and the call's own logging state
#[derive(Debug)]
pub struct VideoReply<'reply> {
    pub raw_response: &'reply UpstreamResponse,
    pub logging_obj: &'reply mut LoggingState,
    pub custom_llm_provider: Option<&'reply str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusMethod {
    Get,
    Post,
}

// The send a status call makes: a JSON body only for a POST, never a timeout
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRequest {
    pub method: StatusMethod,
    pub url: String,
    pub headers: Headers,
    pub body: Option<Params>,
}

// LiteLLM's client kwarg: the handler uses it only when its kind matches the call's own
pub enum SuppliedClient<'call, Synchronous, Asynchronous> {
    Sync(&'call Synchronous),
    Async(&'call Asynchronous),
}

impl<'call, Synchronous, Asynchronous> SuppliedClient<'call, Synchronous, Asynchronous> {
    pub(crate) const fn synchronous(&self) -> Option<&'call Synchronous> {
        match self {
            Self::Sync(client) => Some(*client),
            Self::Async(_) => None,
        }
    }

    pub(crate) const fn asynchronous(&self) -> Option<&'call Asynchronous> {
        match self {
            Self::Sync(_) => None,
            Self::Async(client) => Some(*client),
        }
    }
}

pub struct StatusCall<'call, Synchronous, Asynchronous> {
    pub video_id: &'call str,
    pub custom_llm_provider: &'call str,
    pub litellm_params: &'call Params,
    pub logging: &'call mut LoggingState,
    pub extra_headers: Option<&'call Headers>,
    // accepted to match LiteLLM's signature, which never merges it into the request
    pub extra_body: Option<&'call Params>,
    // accepted to match LiteLLM's signature, which never sends it; the chosen client keeps its own deadline
    pub timeout: Option<Duration>,
    pub api_key: Option<&'call SecretString>,
    pub client: Option<SuppliedClient<'call, Synchronous, Asynchronous>>,
}

// The sync entry point's answer: the result, or the async call the _is_async flag asked it to return
pub enum VideoStatusHandled<Output, Pending> {
    Ready(Output),
    Pending(Pending),
}

// The call a flagged sync entry leaves pending, Send because the async entry point awaits it
pub type PendingStatus<'call> =
    Pin<Box<dyn Future<Output = Result<VideoObject, VideoStatusError>> + Send + 'call>>;
