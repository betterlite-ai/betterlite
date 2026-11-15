#![cfg(test)]

use super::support::{ENTRA, Reply, fixture, payload};

#[tokio::test]
async fn a_flush_sends_every_queued_event_and_clears_it() {
    let fixture = fixture(
        &ENTRA,
        &[
            Reply::TOKEN,
            Reply::OK,
            Reply::OK,
            Reply::OK,
            Reply::OK,
            Reply::OK,
            Reply::OK,
        ],
    );
    let _first = fixture.logger.queue.enqueue(payload("id-1"));
    let _second = fixture.logger.queue.enqueue(payload("id-2"));
    let report = fixture.logger.flush_queue().await;
    assert_eq!(report.batch, 2);
    assert_eq!(report.delivered, 2);
    assert_eq!(report.requeued, 0);
    assert!(fixture.logger.queue.pending().is_empty());
    assert_eq!(fixture.transport.calls().len(), 7);
    assert!(fixture.logger.last_flush_time().as_secs() >= 1_700_000_000);
}

#[tokio::test]
async fn async_send_batch_uploads_each_queued_event_once() {
    let fixture = fixture(&ENTRA, &[Reply::TOKEN]);
    let _first = fixture.logger.queue.enqueue(payload("id-1"));
    let _second = fixture.logger.queue.enqueue(payload("id-2"));
    assert_eq!(fixture.logger.async_send_batch().await, 2);
    assert_eq!(fixture.transport.calls().len(), 7);
}

#[tokio::test]
async fn an_empty_flush_sends_nothing() {
    let fixture = fixture(&ENTRA, &[]);
    let report = fixture.logger.flush_queue().await;
    assert_eq!(report.batch, 0);
    assert!(fixture.transport.calls().is_empty());
    assert_eq!(fixture.logger.async_send_batch().await, 0);
}

#[tokio::test]
async fn the_batch_size_does_not_trigger_a_flush() {
    let variables = [
        ("AZURE_STORAGE_ACCOUNT_NAME", "acct"),
        ("AZURE_STORAGE_FILE_SYSTEM", "logs"),
        ("AZURE_STORAGE_TENANT_ID", "tenant-1"),
        ("AZURE_STORAGE_CLIENT_ID", "client-1"),
        ("AZURE_STORAGE_CLIENT_SECRET", "secret-1"),
        ("DEFAULT_BATCH_SIZE", "1"),
    ];
    let fixture = fixture(&variables, &[]);
    let _enqueued = fixture.logger.queue.enqueue(payload("id-1"));
    assert_eq!(fixture.logger.queue.pending().len(), 1);
    assert!(fixture.transport.calls().is_empty());
}
