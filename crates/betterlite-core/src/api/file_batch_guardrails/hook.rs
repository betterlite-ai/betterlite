use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use super::record_json::RecordObject;
use crate::routes::CallType;

// The keywords the batch scan passes ProxyLogging.pre_call_hook for one record
pub struct PreCallInput<'call> {
    pub user_api_key_dict: &'call UserAPIKeyAuth,
    pub data: RecordObject,
    pub call_type: CallType,
    pub guardrails_only: bool,
}

// betterlite-server implements this over its pre-call chain, which lives above this crate
#[async_trait]
pub trait GuardrailsOnlyPreCall: Sync {
    type Failure: RaisedGuardrail + Send;

    // The body the chain hands back, which may be a replacement for the one it was given
    async fn pre_call_hook(&self, input: PreCallInput<'_>) -> Result<RecordObject, Self::Failure>;
}

// What the batch scan reads from an exception the chain raised
pub trait RaisedGuardrail {
    // GuardrailRaisedException.blocked_content, and None for every other exception class
    fn blocked_content(&self) -> Option<bool>;

    // Whether the exception was raised from another one; the implicit context never counts
    fn has_cause(&self) -> bool;

    // is_guardrail_intervention, which the guardrail failure type owns
    fn is_intervention(&self) -> bool;

    // A str guardrail_name attribute on the exception
    fn guardrail_name(&self) -> Option<&str>;

    // A str guardrail_name inside a dict detail
    fn detail_guardrail_name(&self) -> Option<&str>;
}

// Stricter than is_guardrail_intervention: a fail-closed guardrail raises the same class when it could not judge, and dropping that record would lose enforcement silently
#[must_use]
pub fn is_content_block<Failure>(failure: &Failure) -> bool
where
    Failure: RaisedGuardrail,
{
    if let Some(blocked) = failure.blocked_content() {
        return blocked;
    }
    !failure.has_cause() && failure.is_intervention()
}

#[must_use]
pub fn naming_guardrail<Failure>(failure: &Failure) -> Option<String>
where
    Failure: RaisedGuardrail,
{
    if let Some(named) = failure.guardrail_name() {
        return Some(named.to_owned());
    }
    failure.detail_guardrail_name().map(str::to_owned)
}
