use std::collections::BTreeMap;
use std::future::Future;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::failure::UndeliverableStreamRewrite;
use super::shapes::{RequestData, StreamInputs};

pub type UserApiKey = BTreeMap<String, GuardrailValue>;

pub trait RewriteFailure: From<UndeliverableStreamRewrite> + Send {
    fn undeliverable(&self) -> Option<&UndeliverableStreamRewrite>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputType {
    Request,
    Response,
}

pub struct GuardrailCall<'call, Log> {
    pub request_data: &'call mut RequestData,
    pub input_type: InputType,
    pub logging_obj: Option<&'call mut Log>,
}

pub trait NamedGuardrail {
    fn guardrail_name(&self) -> Option<&str>;
}

// What a translation hands the guardrail it scans with; LiteLLM's CustomGuardrail.apply_guardrail
pub trait StreamGuardrail<Log: Send>: NamedGuardrail + Send {
    type Failure: RewriteFailure;

    fn covers_full_request(&self) -> bool;

    fn apply_guardrail(
        &mut self,
        inputs: StreamInputs,
        call: GuardrailCall<'_, Log>,
    ) -> impl Future<Output = Result<StreamInputs, Self::Failure>> + Send;
}

// LiteLLM reads these from type(callback).__dict__ and from comparisons with CustomLogger's methods
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookSurface {
    pub apply: ApplySurface,
    pub post_call: PostCallSurface,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplySurface {
    pub own_apply_guardrail: bool,
    pub use_native_lifecycle_hooks: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostCallSurface {
    pub own_success_hook: bool,
    pub own_streaming_iterator_hook: bool,
}

pub struct PostCallHook<'hook> {
    pub data: &'hook mut RequestData,
    pub user_api_key_dict: &'hook UserApiKey,
    pub response: &'hook mut Option<GuardrailValue>,
}

pub trait GuardrailCallback<Log: Send>: StreamGuardrail<Log> {
    fn hook_surface(&self) -> HookSurface;

    fn records_own_information(&self) -> bool;

    // a hook that changes the response in place does so through call.response and returns None
    fn async_post_call_success_hook(
        &mut self,
        call: PostCallHook<'_>,
    ) -> impl Future<Output = Result<Option<GuardrailValue>, Self::Failure>> + Send;

    fn supports_unified_execution(&self) -> bool {
        let surface = self.hook_surface().apply;
        surface.own_apply_guardrail && !surface.use_native_lifecycle_hooks
    }

    fn supports_streaming_execution(&self) -> bool {
        let surface = self.hook_surface().post_call;
        self.supports_unified_execution()
            || (surface.own_success_hook && !surface.own_streaming_iterator_hook)
    }
}

pub struct StreamCall<'call, Log> {
    pub request_data: Option<&'call mut RequestData>,
    pub user_api_key_dict: &'call UserApiKey,
    pub logging_obj: Option<&'call mut Log>,
    pub deliver_ended_stream_rewrites: bool,
}

// The endpoint translation's side of the step; LiteLLM's BaseTranslation
pub trait StreamTranslation<Log: Send>: Sync {
    type Failure: RewriteFailure;

    const DELIVERS_ENDED_STREAM_REWRITES: bool = false;

    fn post_call_hook_response(&self, response: Option<GuardrailValue>) -> Option<GuardrailValue> {
        response
    }

    fn process_output_response<Guardrail>(
        &self,
        response: GuardrailValue,
        guardrail: &mut Guardrail,
        call: StreamCall<'_, Log>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send
    where
        Guardrail: StreamGuardrail<Log, Failure = Self::Failure>;

    fn process_streaming_response<Guardrail>(
        &self,
        responses_so_far: &mut Vec<GuardrailValue>,
        guardrail: &mut Guardrail,
        call: StreamCall<'_, Log>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send
    where
        Guardrail: StreamGuardrail<Log, Failure = Self::Failure>;
}

// One litellm.callbacks entry; only CustomGuardrail instances are guardrails
pub trait CallbackEntry {
    type Guardrail: NamedGuardrail;

    fn as_guardrail(&self) -> Option<&Self::Guardrail>;
}
