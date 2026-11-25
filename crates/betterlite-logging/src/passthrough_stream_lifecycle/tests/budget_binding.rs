#![cfg(test)]

use std::future::Future as _;
use std::sync::{Arc, Mutex};
use std::task::Context;

use futures_util::StreamExt as _;

use crate::passthrough_stream_lifecycle::chunk_processor::chunk_processor;
use crate::passthrough_stream_lifecycle::ports::{
    StreamLoggingJob, StreamLoggingQueue, StreamRoute, StreamWork,
};
use crate::passthrough_stream_lifecycle::support::{
    Harness, callback_bound, capture_logs, failing_chunks, failure, json, object, ok_chunks,
    processor_call, reservation_params,
};
use crate::passthrough_stream_logging::build::StreamLoggingInput;
use crate::passthrough_stream_logging::types::EndpointType;
use crate::worker::Admission;

#[derive(Default)]
struct PollingQueue {
    jobs: Mutex<Vec<StreamLoggingJob>>,
    first_poll_pending: Mutex<Option<bool>>,
}

impl StreamLoggingQueue for PollingQueue {
    fn submit(&self, mut job: StreamLoggingJob) -> Admission {
        let waker = futures_util::task::noop_waker();
        let mut context = Context::from_waker(&waker);
        let pending = job.work.as_mut().poll(&mut context).is_pending();
        *self
            .first_poll_pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pending);
        self.jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(job);
        Admission::Accepted
    }
}

impl PollingQueue {
    async fn drain(&self) {
        let jobs = std::mem::take(
            &mut *self
                .jobs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for job in jobs {
            job.work.await;
        }
    }
}

#[derive(Default)]
struct ObservingRoute {
    bound_on_dispatch: Mutex<Vec<Option<betterlite_models::decisions::Json>>>,
}

impl StreamRoute for ObservingRoute {
    fn route(self: Arc<Self>, input: StreamLoggingInput) -> StreamWork {
        Box::pin(async move {
            self.bound_on_dispatch
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(callback_bound(input.logging_obj.as_ref()));
        })
    }
}

#[tokio::test]
async fn a_successful_enqueue_binds_the_reservation() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], reservation_params());
    let call = processor_call(
        Arc::clone(&lifecycle),
        Arc::clone(&logging_obj),
        EndpointType::OpenAi,
    );
    drop(
        chunk_processor(call, ok_chunks(vec![b"alpha".to_vec()]))
            .collect::<Vec<_>>()
            .await,
    );
    assert_eq!(harness.binder.calls(), 1);
    assert_eq!(callback_bound(logging_obj.as_ref()), Some(json("true")));
}

#[tokio::test]
async fn a_rejected_enqueue_never_binds_the_reservation() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]).with_rejecting_queue();
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], reservation_params());
    let call = processor_call(
        Arc::clone(&lifecycle),
        Arc::clone(&logging_obj),
        EndpointType::OpenAi,
    );
    drop(
        chunk_processor(call, ok_chunks(vec![b"alpha".to_vec()]))
            .collect::<Vec<_>>()
            .await,
    );
    assert_eq!(harness.binder.calls(), 0);
    assert!(callback_bound(logging_obj.as_ref()).is_none());
}

#[tokio::test]
async fn a_failure_never_binds_the_reservation() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], reservation_params());
    let call = processor_call(
        Arc::clone(&lifecycle),
        Arc::clone(&logging_obj),
        EndpointType::OpenAi,
    );
    drop(
        chunk_processor(call, failing_chunks(vec![b"alpha".to_vec()], failure()))
            .collect::<Vec<_>>()
            .await,
    );
    assert_eq!(harness.binder.calls(), 0);
    assert!(callback_bound(logging_obj.as_ref()).is_none());
}

#[tokio::test]
async fn a_disconnect_binds_after_the_partial_enqueue() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], reservation_params());
    let call = processor_call(
        Arc::clone(&lifecycle),
        Arc::clone(&logging_obj),
        EndpointType::OpenAi,
    );
    let mut stream = Box::pin(chunk_processor(
        call,
        ok_chunks(vec![b"alpha".to_vec(), b"beta".to_vec()]),
    ));
    assert_eq!(stream.next().await.unwrap().unwrap(), b"alpha");
    drop(stream);
    assert_eq!(harness.binder.calls(), 1);
    assert_eq!(callback_bound(logging_obj.as_ref()), Some(json("true")));
}

#[tokio::test]
async fn a_rejected_partial_enqueue_never_binds() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]).with_rejecting_queue();
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], reservation_params());
    let call = processor_call(
        Arc::clone(&lifecycle),
        Arc::clone(&logging_obj),
        EndpointType::OpenAi,
    );
    let (logs, guard) = capture_logs();
    let mut stream = Box::pin(chunk_processor(
        call,
        ok_chunks(vec![b"alpha".to_vec(), b"beta".to_vec()]),
    ));
    assert_eq!(stream.next().await.unwrap().unwrap(), b"alpha");
    drop(stream);
    drop(guard);
    assert_eq!(harness.binder.calls(), 0);
    assert!(callback_bound(logging_obj.as_ref()).is_none());
    let text = logs.text();
    assert!(
        text.contains("Error scheduling chunk_processor logging: QueueFull"),
        "{text}"
    );
}

#[tokio::test]
async fn an_accepted_job_polled_concurrently_waits_for_reservation_binding() {
    let harness = Harness::new(&["2024-01-01T00:00:01+00:00"]);
    let mut lifecycle = harness.lifecycle(false);
    let queue = Arc::new(PollingQueue::default());
    let route = Arc::new(ObservingRoute::default());
    let current = Arc::get_mut(&mut lifecycle).expect("the lifecycle is uniquely owned");
    let queue_port: Arc<dyn StreamLoggingQueue> = Arc::clone(&queue);
    let route_port: Arc<dyn StreamRoute> = Arc::clone(&route);
    current.queue = queue_port;
    current.route = route_port;
    let logging_obj = object(&[], reservation_params());
    let call = processor_call(
        Arc::clone(&lifecycle),
        Arc::clone(&logging_obj),
        EndpointType::OpenAi,
    );
    drop(
        chunk_processor(call, ok_chunks(vec![b"alpha".to_vec()]))
            .collect::<Vec<_>>()
            .await,
    );

    assert_eq!(
        *queue
            .first_poll_pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        Some(true)
    );
    assert_eq!(harness.binder.calls(), 1);
    assert_eq!(callback_bound(logging_obj.as_ref()), Some(json("true")));
    queue.drain().await;
    assert_eq!(
        *route
            .bound_on_dispatch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![Some(json("true"))]
    );
}
