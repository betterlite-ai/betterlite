#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use indexmap::IndexMap;

use crate::passthrough_stream_lifecycle::ports::{
    StreamJobKind, StreamLoggingJob, StreamLoggingQueue, StreamRoute, StreamWork,
};
use crate::passthrough_stream_lifecycle::support::{
    FakeProbe, Harness, RecordingTinyfish, json, object, passthrough, time,
};
use crate::passthrough_stream_logging::build::StreamLoggingInput;
use crate::passthrough_stream_logging::router::StreamLoggingRouter;
use crate::passthrough_stream_logging::types::EndpointType;
use crate::worker::{Admission, Callback as _, LoggingWorker, Rejection, WorkerSettings};

#[test]
fn stream_jobs_name_their_kind() {
    let success_work: StreamWork = Box::pin(async {});
    let failure_work: StreamWork = Box::pin(async {});
    let success = StreamLoggingJob {
        kind: StreamJobKind::Success,
        work: success_work,
    };
    let failure = StreamLoggingJob {
        kind: StreamJobKind::Failure,
        work: failure_work,
    };
    assert_eq!(success.name(), "passthrough_stream_success");
    assert_eq!(failure.name(), "passthrough_stream_failure");
}

#[tokio::test]
async fn calling_a_stream_job_awaits_its_work() {
    let ran = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&ran);
    let work: StreamWork = Box::pin(async move {
        flag.store(true, Ordering::SeqCst);
    });
    let job = StreamLoggingJob {
        kind: StreamJobKind::Success,
        work,
    };
    job.call().await.unwrap();
    assert!(ran.load(Ordering::SeqCst));
}

#[tokio::test]
async fn the_worker_impl_reports_the_admission() {
    let worker = LoggingWorker::<StreamLoggingJob>::start(WorkerSettings::default())
        .expect("the worker starts in the test runtime");
    let first: StreamWork = Box::pin(async {});
    let accepted = StreamLoggingQueue::submit(
        &worker,
        StreamLoggingJob {
            kind: StreamJobKind::Success,
            work: first,
        },
    );
    assert_eq!(accepted, Admission::Accepted);
    worker.stop().await;
    let second: StreamWork = Box::pin(async {});
    let rejected = StreamLoggingQueue::submit(
        &worker,
        StreamLoggingJob {
            kind: StreamJobKind::Failure,
            work: second,
        },
    );
    assert_eq!(rejected, Admission::Rejected(Rejection::Closed));
}

#[tokio::test]
async fn the_router_route_reaches_the_handler() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let router = Arc::new(StreamLoggingRouter {
        providers: Arc::clone(&harness.providers),
        frames: Arc::<FakeProbe>::clone(&harness.probe),
        tinyfish: Arc::<RecordingTinyfish>::clone(&harness.tinyfish),
    });
    let input = StreamLoggingInput {
        logging_obj: object(&[], IndexMap::new()),
        passthrough: passthrough(),
        providers: Arc::clone(&harness.providers),
        url_route: "/v1/chat/completions".to_owned(),
        request_body: Some(json("{\"model\":\"gpt-4o\"}")),
        endpoint_type: EndpointType::OpenAi,
        start_time: time("2024-01-01T00:00:00+00:00"),
        end_time: time("2024-01-01T00:00:01+00:00"),
        raw_bytes: vec![b"data: {}\n\n".to_vec()],
        model: None,
    };
    StreamRoute::route(router, input).await;
    assert_eq!(harness.handler.called(), 1);
}
