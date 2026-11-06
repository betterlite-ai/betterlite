#![cfg(test)]

use super::support::{LagoBody, logger, logger_with};
use crate::finops::metering::support::{FakeSender, event};

#[tokio::test]
async fn init_posts_through_the_sender_it_was_built_with() {
    let sender = FakeSender::default();
    let logger = logger(sender.clone());
    logger.log_success_event(&event()).await.unwrap();
    logger.async_log_success_event(&event()).await.unwrap();
    assert_eq!(sender.posts().len(), 2);
}

#[tokio::test]
async fn init_keeps_the_base_url_and_event_code_from_the_environment() {
    let sender = FakeSender::default();
    let logger = logger_with(
        sender.clone(),
        &[
            ("LAGO_API_BASE", "https://billing.test/"),
            ("LAGO_API_EVENT_CODE", "tokens"),
        ],
    );
    logger.log_success_event(&event()).await.unwrap();
    let posted = sender.only_post();
    assert_eq!(posted.url, "https://billing.test/api/v1/events");
    let body: LagoBody = posted.parse();
    assert_eq!(body.event.code, "tokens");
}
