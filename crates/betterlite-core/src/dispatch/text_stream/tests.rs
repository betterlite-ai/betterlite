#![cfg(test)]

use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use betterlite_context::call_context::{CallContext, CallInputs};
use betterlite_openai::text_completion_request::CompletionRequest;
use uuid::Uuid;

use crate::dispatch::error::DispatchError;
use crate::dispatch::log::TracingCallLog;
use crate::dispatch::{Dispatch, NoRules};
use crate::operation::Operation;
use crate::operation::text_completion::{
    TextCompletionCall, TextCompletionCallInput, TextCompletionFailure, TextCompletionRequest,
    TextCompletionResult,
};

struct NeverCalled;

#[async_trait]
impl TextCompletionCall for NeverCalled {
    async fn complete(
        &self,
        _call: TextCompletionCallInput<'_>,
    ) -> Result<TextCompletionResult, TextCompletionFailure> {
        Err(TextCompletionFailure {
            status: 500,
            error_type: None,
            param: None,
            message: "the stub never reaches the provider".to_owned(),
        })
    }
}

#[tokio::test]
async fn call_dispatch_tests_behavior() {
    let dispatch = Arc::new(Dispatch::new(
        Vec::new(),
        Arc::new(NoRules),
        Arc::new(TracingCallLog),
    ));
    let context = CallContext::start(CallInputs {
        now: || SystemTime::UNIX_EPOCH,
        id: Uuid::nil,
        deadline: None,
        privacy: false,
    });
    let request = TextCompletionRequest {
        call_id: "call".to_owned(),
        model_id: "model".to_owned(),
        request: Box::new(
            CompletionRequest::parse(br#"{"model":"g","prompt":"hello"}"#)
                .expect("completion request"),
        ),
        dispatch: Arc::clone(&dispatch),
        calls: Arc::new(NeverCalled),
    };
    let result = dispatch
        .execute(Operation::TextStream(request), &context)
        .await;
    assert!(matches!(
        result,
        Err(DispatchError::NotBuilt {
            entry: "text_stream"
        })
    ));
}
