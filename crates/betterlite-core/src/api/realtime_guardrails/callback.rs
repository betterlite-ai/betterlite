use betterlite_config::sections::guardrail_params::base::OnViolation;
use betterlite_models::decisions::Json;
use futures_util::future::LocalBoxFuture;

use crate::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use crate::api::request_destination::RequestData;

// What LiteLLM hands a CustomGuardrail.apply_guardrail for realtime text
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardrailInput {
    pub texts: Vec<String>,
    pub images: Vec<Json>,
    pub request_data: RequestData,
    pub input_type: GuardrailInputType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardrailInputType {
    Request,
    Response,
}

// apply_guardrail's rewritten inputs; LiteLLM ignores them for realtime, so the runner drops them
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuardrailTexts(pub Vec<String>);

// The two exception attributes LiteLLM reads after a CustomGuardrail raises
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardrailViolation {
    // getattr(exception, "detail", None); Json::Null when the exception has no detail attribute
    pub detail: Json,
    // str(exception)
    pub message: String,
}

// LiteLLM blocks on a raised exception with a status_code or a ValueError; detail alone is an unexpected error that is re-raised
#[derive(Debug)]
pub enum GuardrailFailure<CallbackError> {
    Blocked(GuardrailViolation),
    Unexpected(CallbackError),
}

// The realtime attributes LiteLLM reads from the guardrail object that blocked
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RealtimeGuardrailPolicy {
    // realtime_violation_message; an empty one falls through to the raised message
    pub violation_message: Option<String>,
    pub on_violation: Option<OnViolation>,
    // end_session_after_n_fails; 0 closes the session on the first violation
    pub end_session_after_n_fails: Option<usize>,
}

pub type GuardrailApplication<'call, CallbackError> =
    LocalBoxFuture<'call, Result<GuardrailTexts, GuardrailFailure<CallbackError>>>;

// B2455's detection and predicate trait plus the apply_guardrail surface and the block attributes the realtime runner reads
pub trait RealtimeGuardrail: RealtimeGuardrailCallback {
    fn use_native_lifecycle_hooks(&self) -> bool;

    fn policy(&self) -> &RealtimeGuardrailPolicy;

    fn apply_guardrail(&self, input: GuardrailInput) -> GuardrailApplication<'_, Self::Error>;
}
