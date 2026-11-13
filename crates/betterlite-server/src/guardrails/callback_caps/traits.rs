use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::routes::CallType;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::errors::{GuardrailGateError, HttpException, ProxyHookError};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RequestHooks {
    pub pre_call: bool,
    pub moderation: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PostCallHooks {
    pub success: bool,
    pub failure: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HeaderHooks {
    pub response_headers: bool,
    pub accepts_call_info: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StreamingHooks {
    pub chunk: bool,
    pub iterator: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HookOverrides {
    pub apply_guardrail: bool,
    pub filter_listed_models: bool,
    pub request: RequestHooks,
    pub post_call: PostCallHooks,
    pub headers: HeaderHooks,
    pub streaming: StreamingHooks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreCallReturn {
    Unchanged,
    Data(JsonMap),
    Text(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallInfo {
    pub custom_llm_provider: Option<String>,
    pub model_info: Option<JsonMap>,
    pub api_base: Option<String>,
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HeadersContext<'request> {
    pub request_headers: Option<&'request BTreeMap<String, String>>,
    pub call_info: Option<&'request CallInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureReply {
    Original,
    Replace(HttpException),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardrailEvent {
    PreCall,
    PostCall,
    DuringCall,
    LoggingOnly,
    PreMcpCall,
    DuringMcpCall,
    PostMcpCall,
    RealtimeInputTranscription,
}

#[async_trait]
pub trait ProxyCallback: Send + Sync {
    fn overrides(&self) -> HookOverrides;
    fn class_name(&self) -> &str;
    fn is_base_logger(&self) -> bool;
    fn enforces_request_content(&self) -> bool;
    fn use_native_lifecycle_hooks(&self) -> bool;
    fn guardrail(&self) -> Option<&dyn GuardrailCallback>;

    async fn pre_call(
        &self,
        _user_api_key_dict: &UserAPIKeyAuth,
        _data: &mut JsonMap,
        _call_type: CallType,
    ) -> Result<PreCallReturn, ProxyHookError> {
        Ok(PreCallReturn::Unchanged)
    }

    async fn moderation(
        &self,
        _data: &mut JsonMap,
        _user_api_key_dict: Option<&UserAPIKeyAuth>,
        _call_type: CallType,
    ) -> Result<(), ProxyHookError> {
        Ok(())
    }

    async fn post_call_success(
        &self,
        _data: &mut JsonMap,
        _user_api_key_dict: Option<&UserAPIKeyAuth>,
        _response: JsonValue,
    ) -> Result<Option<JsonValue>, ProxyHookError> {
        Ok(None)
    }

    async fn post_call_failure(
        &self,
        _request_data: &mut JsonMap,
        _original_exception: &ProxyHookError,
        _user_api_key_dict: &UserAPIKeyAuth,
        _traceback: Option<&str>,
    ) -> Result<FailureReply, ProxyHookError> {
        Ok(FailureReply::Original)
    }

    async fn response_headers(
        &self,
        _data: &JsonMap,
        _user_api_key_dict: &UserAPIKeyAuth,
        _response: &JsonValue,
        _context: &HeadersContext<'_>,
    ) -> Result<BTreeMap<String, String>, ProxyHookError> {
        Ok(BTreeMap::new())
    }

    async fn streaming_chunk(
        &self,
        _user_api_key_dict: &UserAPIKeyAuth,
        _response: &str,
    ) -> Result<Option<JsonValue>, ProxyHookError> {
        Ok(None)
    }

    async fn filter_listed_models(
        &self,
        _user_api_key_dict: &UserAPIKeyAuth,
        model_names: Vec<String>,
    ) -> Result<Vec<String>, ProxyHookError> {
        Ok(model_names)
    }
}

pub trait GuardrailCallback: Send + Sync {
    fn guardrail_name(&self) -> Option<&str>;
    fn run_in_parallel(&self) -> bool;
    fn scan_raw_request(&self) -> bool;
    fn use_native_during_call_hook(&self) -> bool;
    fn event_hook_is_set(&self) -> bool;
    fn moderation_check(&self) -> Option<&str>;
    fn should_run_guardrail(
        &self,
        data: &JsonMap,
        event: GuardrailEvent,
    ) -> Result<bool, GuardrailGateError>;
    fn mark_pre_call_hook_ran(&self, data: &mut JsonMap);
}
