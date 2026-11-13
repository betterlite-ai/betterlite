#![cfg(test)]

use super::support::{API_KEY, CloudEventBody, logger};
use crate::finops::metering::openmeter::OpenMeterLogger;
use crate::finops::metering::openmeter::settings::{DEFAULT_EVENT_TYPE, OpenMeterSettings};
use crate::finops::metering::support::{FakeSender, FixedClock, event, variables};

#[tokio::test]
async fn init_posts_through_the_sender_it_was_built_with() {
    let sender = FakeSender::default();
    let logger = logger(sender.clone());
    logger.log_success_event(&event()).await.unwrap();
    logger.async_log_success_event(&event()).await.unwrap();
    assert_eq!(sender.posts().len(), 2);
}

#[tokio::test]
async fn init_defaults_the_endpoint_and_event_type() {
    let sender = FakeSender::default();
    let environment = variables(&[("OPENMETER_API_KEY", API_KEY)]);
    let settings = OpenMeterSettings::from_environment(&environment).unwrap();
    let logger = OpenMeterLogger {
        settings,
        sender: sender.clone(),
        clock: FixedClock,
    };
    logger.log_success_event(&event()).await.unwrap();
    let posted = sender.only_post();
    assert_eq!(posted.url, "https://openmeter.cloud/api/v1/events");
    assert_eq!(posted.content_type, "application/cloudevents+json");
    let body: CloudEventBody = posted.parse();
    assert_eq!(body.specversion, "1.0");
    assert_eq!(body.event_type, DEFAULT_EVENT_TYPE);
    assert_eq!(body.source, "litellm-proxy");
}
