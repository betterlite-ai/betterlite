#![cfg(test)]

use std::sync::Arc;
use std::time::SystemTime;

use betterlite_context::call_context::{CallContext, CallInputs};
use uuid::Uuid;

use crate::dispatch::error::DispatchError;
use crate::dispatch::log::TracingCallLog;
use crate::dispatch::{Dispatch, NoRules};
use crate::operation::{Operation, responses::ResponsesRequest};

#[tokio::test]
async fn call_dispatch_tests_behavior() {
    let dispatch = Dispatch::new(Vec::new(), Arc::new(NoRules), Arc::new(TracingCallLog));
    let context = CallContext::start(CallInputs {
        now: || SystemTime::UNIX_EPOCH,
        id: Uuid::nil,
        deadline: None,
        privacy: false,
    });
    let result = dispatch
        .execute(Operation::Responses(ResponsesRequest), &context)
        .await;
    assert!(matches!(
        result,
        Err(DispatchError::NotBuilt { entry: "responses" })
    ));
}
