use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::sse_failure;
use crate::first_byte_sse::{HookRaised, LateFailureHook};
use crate::invitation_routes::state::QueueFailureHook;
use crate::sse_error_frames::{SseFailure, exception_detail_parts};

mod tests;

pub(super) struct QueueLateFailure {
    pub(super) hook: Arc<dyn QueueFailureHook>,
    pub(super) caller: Principal,
    pub(super) data: JsonMap,
}

#[async_trait]
impl LateFailureHook for QueueLateFailure {
    async fn late_failure(&self, failure: &SseFailure) -> Result<Option<SseFailure>, HookRaised> {
        let exception = exception_from_sse(failure);
        let answered = self
            .hook
            .post_call_failure(&self.caller, &exception, &self.data)
            .await
            .as_ref()
            .map(sse_failure);
        Ok(answered)
    }
}

pub(super) fn exception_from_sse(failure: &SseFailure) -> ProxyException {
    let message = failure
        .detail
        .as_deref()
        .map_or_else(String::new, |detail| exception_detail_parts(detail).0);
    let code = failure.code.clone().map_or_else(
        || {
            failure.status_code.map_or(ExceptionCode::Absent, |status| {
                ExceptionCode::Status(i64::from(status))
            })
        },
        ExceptionCode::Text,
    );
    ProxyException::new(ProxyExceptionInit {
        message,
        error_type: failure
            .error_type
            .clone()
            .unwrap_or_else(|| ProxyErrorTypes::AuthError.to_string()),
        param: failure.param.clone(),
        code,
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}
