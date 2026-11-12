use std::sync::Arc;
use std::time::{Duration, SystemTime};

use betterlite_context::call_context::CallContext;
use betterlite_openai::chat::ChatRequest as ClientChatRequest;
use tokio::task::JoinSet;

use super::support::{
    FakeChat, MODEL_ID, RecordedCall, RecordingLog, StreamEnd, Trace, context, deployment,
};
use crate::batch::execution::{Batch, BatchLimits, Duplicate, Item, Mode};
use crate::dispatch::error::DispatchError;
use crate::dispatch::log::CallOutcome;
use crate::dispatch::{Dispatch, NoRules};
use crate::operation::chat::{ChatCall, ChatRequest, ChatResult};
use crate::operation::{CallResult, Operation};

type PlannedCall = (CallContext<'static>, String);
type BatchReport = Vec<Result<Option<CallResult>, DispatchError>>;

fn planned_batch() -> Batch<ClientChatRequest, PlannedCall> {
    let items = ["call-a", "call-b"]
        .into_iter()
        .map(|call_id| Item {
            key: call_id.to_owned(),
            request: ClientChatRequest::parse(br#"{"model":"g","messages":[]}"#)
                .expect("batch request"),
            context: (context(None, SystemTime::UNIX_EPOCH), call_id.to_owned()),
        })
        .collect();
    Batch {
        items,
        limits: BatchLimits {
            max_items: 4,
            max_bytes: 8192,
            concurrency: 2,
            deadline: Duration::from_secs(5),
        },
        mode: Mode::Ordered,
    }
}

async fn run_batch() -> (BatchReport, Vec<Duplicate>, Vec<RecordedCall>) {
    let trace = Trace::default();
    let calls: Arc<dyn ChatCall> = Arc::new(FakeChat::new(trace.clone(), StreamEnd::Ends));
    let log = Arc::new(RecordingLog::new(trace));
    let dispatch = Arc::new(Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(1))],
        Arc::new(NoRules),
        Arc::clone(&log),
    ));
    let mut owners = JoinSet::new();
    let receiver = planned_batch()
        .start(
            move |request: ClientChatRequest, (context, call_id): PlannedCall| {
                let dispatch_for_call = Arc::clone(&dispatch);
                let calls_for_call = Arc::clone(&calls);
                async move {
                    let call = chat_call(call_id, request, calls_for_call);
                    dispatch_for_call.execute(call, &context).await.map(Some)
                }
            },
            &mut owners,
        )
        .expect("batch starts");
    let report = receiver
        .await
        .expect("batch completes")
        .expect("batch report");
    while let Some(joined) = owners.join_next().await {
        joined.expect("batch owner task");
    }
    (report.results, report.duplicates, log.calls())
}

fn chat_call(call_id: String, request: ClientChatRequest, calls: Arc<dyn ChatCall>) -> Operation {
    Operation::Chat(ChatRequest {
        call_id,
        model_id: String::from(MODEL_ID),
        request: Box::new(request),
        calls,
    })
}

#[tokio::test]
async fn call_dispatch_tests_behavior() {
    let (results, duplicates, recorded) = run_batch().await;
    assert!(duplicates.is_empty());
    assert_eq!(results.len(), 2);
    assert!(matches!(
        results.as_slice(),
        [
            Ok(Some(CallResult::Chat(ChatResult::Completion(_)))),
            Ok(Some(CallResult::Chat(ChatResult::Completion(_)))),
        ]
    ));
    assert_eq!(recorded.len(), 2);
    assert!(
        recorded
            .iter()
            .all(|call| call.outcome == CallOutcome::Succeeded)
    );
}
