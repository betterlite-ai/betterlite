#![cfg(test)]

use super::support::{ENTRA, Reply, fixture, payload};

fn token_reply() -> Reply {
    Reply {
        status: 200,
        body: r#"{"access_token": "tok-1"}"#,
    }
}

#[tokio::test]
async fn a_first_failure_stops_the_batch_and_keeps_every_event() {
    let fixture = fixture(
        &ENTRA,
        &[
            token_reply(),
            Reply {
                status: 500,
                body: "",
            },
        ],
    );
    let _first = fixture.logger.queue.enqueue(payload("id-1"));
    let _second = fixture.logger.queue.enqueue(payload("id-2"));
    let report = fixture.logger.flush_queue().await;
    assert_eq!(report.delivered, 0);
    assert_eq!(report.requeued, 2);
    let queued: Vec<String> = fixture
        .logger
        .queue
        .pending()
        .into_iter()
        .map(|record| record.id)
        .collect();
    assert_eq!(queued, vec!["id-1".to_owned(), "id-2".to_owned()]);
    assert_eq!(fixture.transport.calls().len(), 2);
}

#[tokio::test]
async fn a_middle_failure_keeps_the_failed_event_and_the_rest() {
    let fixture = fixture(
        &ENTRA,
        &[
            token_reply(),
            Reply::OK,
            Reply::OK,
            Reply::OK,
            Reply {
                status: 503,
                body: "",
            },
        ],
    );
    let _first = fixture.logger.queue.enqueue(payload("id-1"));
    let _second = fixture.logger.queue.enqueue(payload("id-2"));
    let _third = fixture.logger.queue.enqueue(payload("id-3"));
    let report = fixture.logger.flush_queue().await;
    assert_eq!(report.delivered, 1);
    assert_eq!(report.requeued, 2);
    let queued: Vec<String> = fixture
        .logger
        .queue
        .pending()
        .into_iter()
        .map(|record| record.id)
        .collect();
    assert_eq!(queued, vec!["id-2".to_owned(), "id-3".to_owned()]);
    assert_eq!(fixture.transport.calls().len(), 5);
}

#[tokio::test]
async fn the_next_flush_retries_only_the_unfinished_events() {
    let fixture = fixture(
        &ENTRA,
        &[
            token_reply(),
            Reply::OK,
            Reply::OK,
            Reply::OK,
            Reply {
                status: 500,
                body: "",
            },
            Reply::OK,
            Reply::OK,
            Reply::OK,
        ],
    );
    let _first = fixture.logger.queue.enqueue(payload("id-1"));
    let _second = fixture.logger.queue.enqueue(payload("id-2"));
    let _first_report = fixture.logger.flush_queue().await;
    assert_eq!(_first_report.requeued, 1);
    let _second_report = fixture.logger.flush_queue().await;
    assert_eq!(_second_report.batch, 1);
    assert_eq!(_second_report.delivered, 1);
    assert!(fixture.logger.queue.pending().is_empty());
    assert_eq!(fixture.transport.calls().len(), 8);
}
