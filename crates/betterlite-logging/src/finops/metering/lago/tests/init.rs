#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;

use super::support::{LagoBody, logger, logger_with};
use crate::finops::metering::support::{FakeSender, event};
use crate::finops::metering::transport::{
    CONNECT_TIMEOUT, DEFAULT_REQUEST_TIMEOUT, HttpMeteringSender,
};

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

#[test]
fn init_http_sender_uses_the_resolved_request_timeout() {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let sender = HttpMeteringSender::new(&policy, Duration::from_secs(30)).unwrap();
    assert_eq!(sender.deadline, Duration::from_secs(30));
    assert_eq!(sender.client.limits.read_timeout, Duration::from_secs(30));
    assert_eq!(sender.client.limits.connect_timeout, CONNECT_TIMEOUT);
    assert_eq!(DEFAULT_REQUEST_TIMEOUT, Duration::from_secs(600));
}
