pub mod client;
pub mod config;
mod convert;
pub mod error;
mod fields;
mod list;
mod logging;
pub mod records;
mod remix;
mod tests;
mod wire;

use std::time::Duration;

use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use secrecy::SecretString;

use self::client::{VideoClient, VideoClientSource};
use self::records::{Headers, Params};

// The sync handler answers with the result, or with the pending async handler when _is_async is set
pub enum Handled<Done, Pending> {
    Done(Done),
    Pending(Pending),
}

// The logging object and the environment the pre-call and a provider transform read
pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

// The B173 pool the four handlers take their clients from; the call site shares one across calls
pub struct VideoRemixListTransport<Source: VideoClientSource> {
    pub source: Source,
}

pub struct RemixCall<'call, Source: VideoClientSource> {
    pub video_id: &'call str,
    pub prompt: &'call str,
    pub custom_llm_provider: &'call str,
    pub litellm_params: &'call Params,
    pub client: Option<VideoClient<'call, Source>>,
    pub extra_headers: Option<&'call Headers>,
    pub extra_body: Option<&'call Params>,
    pub timeout: Option<Duration>,
    pub api_key: Option<&'call SecretString>,
}

pub struct ListCall<'call, Source: VideoClientSource> {
    pub after: Option<&'call str>,
    pub limit: Option<i64>,
    pub order: Option<&'call str>,
    pub custom_llm_provider: &'call str,
    pub litellm_params: &'call Params,
    pub client: Option<VideoClient<'call, Source>>,
    pub extra_headers: Option<&'call Headers>,
    pub extra_query: Option<&'call Params>,
    // LiteLLM's list handler takes the timeout and never sends it; the GET carries no timeout
    pub timeout: Option<Duration>,
    pub api_key: Option<&'call SecretString>,
}
