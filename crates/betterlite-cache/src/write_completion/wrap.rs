use crate::cache_key::value::KeyObject;
use crate::handler::HandlerError;
use crate::handler::call_types::should_store_result_in_cache;
use crate::handler::state::OriginalFunction;
use crate::types::call_type::CachingSupportedCallType;

// What wrap_streaming_result_for_cache can receive: an async stream, a sync stream, or a finished response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResultKind {
    AsyncStream,
    SyncStream,
    Response,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamCacheWrap {
    Wrap,
    PassThrough,
}

// The handler's own fields the decision reads, not the call's merged kwargs.
pub struct StreamCacheHandler<'handler> {
    pub cache_configured: bool,
    pub supported_call_types: Option<&'handler [CachingSupportedCallType]>,
    pub original_function: &'handler OriginalFunction,
    pub request_kwargs: &'handler KeyObject,
}

// ast-grep-ignore: no-long-names - LiteLLM's wrap_streaming_result_for_cache: only an Anthropic Messages async stream that would be stored gets a writer.
pub fn wrap_streaming_result_for_cache(
    result_kind: ResultKind,
    call_type: CachingSupportedCallType,
    handler: &StreamCacheHandler<'_>,
) -> Result<StreamCacheWrap, HandlerError> {
    if !matches!(
        call_type,
        CachingSupportedCallType::AnthropicMessages | CachingSupportedCallType::AanthropicMessages
    ) {
        return Ok(StreamCacheWrap::PassThrough);
    }
    if !handler.cache_configured
        || !should_store_result_in_cache(
            handler.original_function.name.as_str(),
            handler.supported_call_types,
            handler.request_kwargs,
        )?
    {
        return Ok(StreamCacheWrap::PassThrough);
    }
    match result_kind {
        ResultKind::AsyncStream => Ok(StreamCacheWrap::Wrap),
        ResultKind::SyncStream | ResultKind::Response => Ok(StreamCacheWrap::PassThrough),
    }
}
