#![cfg(test)]

mod support;

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Waker};
use std::time::Duration;

use axum::body::to_bytes;
use tokio::sync::Barrier;
use tokio::time::timeout;
use tokio_util::task::TaskTracker;
use tower::{Layer as _, Service as _};
use tracing::subscriber::{DefaultGuard, set_default};

use self::support::{
    CaptureWriter, Handler, InnerFailed, NotReadyOnce, Reply, Stamp, events, handler, logged,
    recorder, release_layer, request, text,
};
use super::BudgetReleaseLayer;
use super::slot::ReservationSlot;

fn capture_logs() -> (Arc<Mutex<String>>, DefaultGuard) {
    let records = Arc::new(Mutex::new(String::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(CaptureWriter(Arc::clone(&records)))
        .finish();
    (records, set_default(subscriber))
}

#[tokio::test]
async fn a_stamped_reservation_is_released_once_after_the_handler_returns() {
    let seen = events();
    let releaser = recorder(&seen, false);
    let mut service = release_layer(&releaser).layer(handler(Reply::Plain, &seen));
    let response = service.call(request(Some(7))).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(text(response).await, "ok");
    assert_eq!(
        releaser.released(),
        [Stamp {
            id: 7,
            callback_bound: false
        }]
    );
    assert_eq!(
        *seen.lock().unwrap(),
        ["handler 7 returned", "released 7"],
        "the release waits for the handler"
    );
}

#[tokio::test]
async fn an_unstamped_request_gets_no_release_and_its_response_unchanged() {
    let seen = events();
    let releaser = recorder(&seen, false);
    let mut service = release_layer(&releaser).layer(handler(Reply::Plain, &seen));
    let response = service.call(request(None)).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(text(response).await, "ok");
    assert!(releaser.released().is_empty());
    assert_eq!(*seen.lock().unwrap(), ["handler none returned"]);
}

#[tokio::test]
async fn a_callback_bound_reservation_still_reaches_the_releaser() {
    let seen = events();
    let releaser = recorder(&seen, false);
    let bound = Handler {
        callback_bound: true,
        ..handler(Reply::Plain, &seen)
    };
    let mut service = release_layer(&releaser).layer(bound);
    service.call(request(Some(3))).await.unwrap();
    assert_eq!(
        releaser.released(),
        [Stamp {
            id: 3,
            callback_bound: true
        }],
        "the skip for bound reservations belongs to the releaser"
    );
}

#[tokio::test]
async fn an_error_response_still_releases_and_is_returned_unchanged() {
    let seen = events();
    let releaser = recorder(&seen, false);
    let mut service = release_layer(&releaser).layer(handler(Reply::ServerError, &seen));
    let response = service.call(request(Some(4))).await.unwrap();
    assert_eq!(response.status(), 500);
    assert_eq!(text(response).await, "upstream broke");
    assert_eq!(*seen.lock().unwrap(), ["handler 4 returned", "released 4"]);
}

#[tokio::test]
async fn an_inner_service_error_still_releases_and_is_returned_unchanged() {
    let seen = events();
    let releaser = recorder(&seen, false);
    let mut service = release_layer(&releaser).layer(handler(Reply::Failed, &seen));
    let error = service.call(request(Some(5))).await.unwrap_err();
    assert_eq!(error, InnerFailed);
    assert_eq!(*seen.lock().unwrap(), ["handler 5 returned", "released 5"]);
}

#[tokio::test]
async fn a_failed_release_is_logged_and_keeps_the_response() {
    let seen = events();
    let releaser = recorder(&seen, true);
    let mut service = release_layer(&releaser).layer(handler(Reply::Plain, &seen));
    let (records, _guard) = capture_logs();
    let response = service.call(request(Some(6))).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(text(response).await, "ok");
    assert_eq!(releaser.released().len(), 1);
    let lines = logged(&records);
    assert!(lines.contains("ERROR"), "{lines}");
    assert!(
        lines.contains("releasing the unclaimed budget reservation failed"),
        "{lines}"
    );
    assert!(
        lines.contains("the spend counters are unreachable"),
        "{lines}"
    );
}

#[tokio::test]
async fn a_successful_release_logs_nothing() {
    let seen = events();
    let mut service = release_layer(&recorder(&seen, false)).layer(handler(Reply::Plain, &seen));
    let (records, _guard) = capture_logs();
    service.call(request(Some(8))).await.unwrap();
    assert_eq!(logged(&records), "");
}

#[tokio::test]
async fn concurrent_requests_each_release_only_their_own_reservation() {
    let seen = events();
    let releaser = recorder(&seen, false);
    let both_stamped = Handler {
        barrier: Some(Arc::new(Barrier::new(2))),
        ..handler(Reply::Plain, &seen)
    };
    let mut first = release_layer(&releaser).layer(both_stamped);
    let mut second = first.clone();
    let (first_reply, second_reply) =
        tokio::join!(first.call(request(Some(1))), second.call(request(Some(2))));
    first_reply.unwrap();
    second_reply.unwrap();
    let mut ids: Vec<u32> = releaser
        .released()
        .into_iter()
        .map(|stamp| stamp.id)
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, [1, 2]);
}

#[tokio::test]
async fn nested_layers_release_the_reservation_once() {
    let seen = events();
    let releaser = recorder(&seen, false);
    let layer = release_layer(&releaser);
    let mut service = layer
        .clone()
        .layer(layer.layer(handler(Reply::Plain, &seen)));
    service.call(request(Some(9))).await.unwrap();
    assert_eq!(*seen.lock().unwrap(), ["handler 9 returned", "released 9"]);
}

#[tokio::test]
async fn a_streaming_response_is_released_when_its_head_returns() {
    let seen = events();
    let releaser = recorder(&seen, false);
    let mut service = release_layer(&releaser).layer(handler(Reply::Streaming, &seen));
    let response = service.call(request(Some(10))).await.unwrap();
    assert_eq!(releaser.released().len(), 1, "released with the head");
    timeout(
        Duration::from_millis(50),
        to_bytes(response.into_body(), 1024),
    )
    .await
    .expect_err("the body is still streaming");
}

#[tokio::test]
async fn poll_ready_waits_for_the_inner_service() {
    let seen = events();
    let mut service = release_layer(&recorder(&seen, false)).layer(NotReadyOnce {
        inner: handler(Reply::Plain, &seen),
        polled: false,
    });
    let mut context = Context::from_waker(Waker::noop());
    assert!(service.poll_ready(&mut context).is_pending());
    assert!(service.poll_ready(&mut context).is_ready());
}

#[tokio::test]
async fn a_dropped_request_still_releases_its_reservation() {
    let seen = events();
    let releaser = recorder(&seen, false);
    let tasks = TaskTracker::new();
    let mut service = BudgetReleaseLayer {
        releaser: Arc::clone(&releaser),
        tasks: tasks.clone(),
    }
    .layer(Handler {
        barrier: Some(Arc::new(Barrier::new(2))),
        ..handler(Reply::Plain, &seen)
    });
    let mut request_future = service.call(request(Some(11)));
    let mut context = Context::from_waker(Waker::noop());
    assert!(request_future.as_mut().poll(&mut context).is_pending());
    drop(request_future);
    tasks.close();
    tasks.wait().await;
    assert_eq!(
        releaser.released(),
        [Stamp {
            id: 11,
            callback_bound: false
        }],
        "the dropped request releases the reservation its handler stamped"
    );
    assert_eq!(
        *seen.lock().unwrap(),
        ["released 11"],
        "the handler never returned"
    );
}

#[test]
fn a_second_stamp_replaces_the_first_and_warns() {
    let (records, _guard) = capture_logs();
    let slot = ReservationSlot::default();
    slot.set(Stamp {
        id: 1,
        callback_bound: false,
    });
    assert_eq!(logged(&records), "", "the first stamp is not a replacement");
    slot.set(Stamp {
        id: 2,
        callback_bound: false,
    });
    let taken = slot.0.lock().unwrap_or_else(PoisonError::into_inner).take();
    assert_eq!(
        taken,
        Some(Stamp {
            id: 2,
            callback_bound: false
        })
    );
    let lines = logged(&records);
    assert!(lines.contains("WARN"), "{lines}");
    assert!(
        lines.contains("replaced the one already stamped for this request"),
        "{lines}"
    );
}
