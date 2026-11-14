#![cfg(test)]

use super::support::{ENTRA, event, fixture};
use crate::callback_logger::CallbackLogger as _;

#[test]
fn callback_logger_reports_the_azure_storage_name() {
    let fixture = fixture(&ENTRA, &[]);
    assert_eq!(fixture.logger.callback_name(), "azure_storage");
}

#[tokio::test]
async fn callback_logger_queues_success_and_failure_events() {
    let fixture = fixture(&ENTRA, &[]);
    fixture
        .logger
        .async_log_success_event(event("id-1"))
        .await
        .expect("queued");
    fixture
        .logger
        .async_log_failure_event(event("id-2"))
        .await
        .expect("queued");
    let queued: Vec<String> = fixture
        .logger
        .queue
        .pending()
        .into_iter()
        .map(|record| record.id)
        .collect();
    assert_eq!(queued, vec!["id-1".to_owned(), "id-2".to_owned()]);
}
