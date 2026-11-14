use rstest::rstest;

use crate::alert_dispatch::AlertLevel;
use crate::outage_alerts::ports::ProviderResolution;

use super::support::{
    BOTH_SENT, FakeFailure, Fixture, MINOR_SENT, NONE_SENT, StoreWrite, deployment_record,
    model_deployment,
};

const MODEL_MINOR_TWO: &str = "\n\n\n*\u{26a0}\u{fe0f} Minor Service Outage*\n\n\n*Model Name:* `gpt-4o`\n*Provider:* `openai`\n*API Base:* `https://api.example.test`\n\n\n*Errors:*\n\nAPI Errors: 2\n\n\n*Last Check:* `0.0s ago`\n\n\n";
const MODEL_MINOR_FIVE: &str = "\n\n\n*\u{26a0}\u{fe0f} Minor Service Outage*\n\n\n*Model Name:* `gpt-4o`\n*Provider:* `openai`\n*API Base:* `https://api.example.test`\n\n\n*Errors:*\n\nAPI Errors: 5\n\n\n*Last Check:* `0.0s ago`\n\n\n";
const MODEL_MINOR_SEVEN: &str = "\n\n\n*\u{26a0}\u{fe0f} Minor Service Outage*\n\n\n*Model Name:* `gpt-4o`\n*Provider:* `openai`\n*API Base:* `https://api.example.test`\n\n\n*Errors:*\n\nAPI Errors: 7\n\n\n*Last Check:* `0.0s ago`\n\n\n";
const MODEL_MINOR_TEN: &str = "\n\n\n*\u{26a0}\u{fe0f} Minor Service Outage*\n\n\n*Model Name:* `gpt-4o`\n*Provider:* `openai`\n*API Base:* `https://api.example.test`\n\n\n*Errors:*\n\nAPI Errors: 10\n\n\n*Last Check:* `0.0s ago`\n\n\n";
const MODEL_MAJOR_FOUR: &str = "\n\n\n*\u{26a0}\u{fe0f} Major Service Outage*\n\n\n*Model Name:* `gpt-4o`\n*Provider:* `openai`\n*API Base:* `https://api.example.test`\n\n\n*Errors:*\n\nAPI Errors: 4\n\n\n*Last Check:* `0.0s ago`\n\n\n";
const MODEL_UNRESOLVED_FIVE: &str = "\n\n\n*\u{26a0}\u{fe0f} Minor Service Outage*\n\n\n*Model Name:* `gpt-4o-mini`\n*Provider:* ``\n*API Base:* `https://api.example.test`\n\n\n*Errors:*\n\nAPI Errors: 5\n\n\n*Last Check:* `0.0s ago`\n\n\n";

#[rstest]
#[case::missing_status(None)]
#[case::bad_request(Some(400))]
#[case::unauthorized(Some(401))]
#[case::rate_limited(Some(429))]
#[case::client_error(Some(499))]
#[tokio::test]
async fn deployment_ignores_a_missing_or_non_qualifying_status(#[case] status_code: Option<i64>) {
    let fixture = Fixture::new(model_deployment());
    fixture.detector().outage_alerts(status_code, "dep-1").await;
    let state = fixture.store.lock();
    assert_eq!(state.reads, ["dep-1".to_owned()]);
    assert!(state.writes.is_empty());
    assert!(state.write_attempts.is_empty());
    drop(state);
    assert!(fixture.deployments.lookups().is_empty());
    assert!(fixture.sender.sent().is_empty());
}

#[tokio::test]
async fn deployment_ignores_a_missing_router_lookup() {
    let mut fixture = Fixture::new(model_deployment());
    fixture.router = false;
    fixture.detector().outage_alerts(Some(500), "dep-1").await;
    let state = fixture.store.lock();
    assert_eq!(state.reads, ["dep-1".to_owned()]);
    assert!(state.writes.is_empty());
    drop(state);
    assert!(fixture.deployments.lookups().is_empty());
    assert!(fixture.sender.sent().is_empty());
}

#[tokio::test]
async fn deployment_ignores_an_unknown_deployment() {
    let mut fixture = Fixture::new(model_deployment());
    fixture.deployments.found = None;
    fixture.detector().outage_alerts(Some(500), "missing").await;
    assert_eq!(fixture.deployments.lookups(), ["missing".to_owned()]);
    let state = fixture.store.lock();
    assert_eq!(state.reads, ["missing".to_owned()]);
    assert!(state.writes.is_empty());
    drop(state);
    assert!(fixture.sender.sent().is_empty());
}

#[rstest]
#[case::timeout(408)]
#[case::server_error(500)]
#[case::above_server_error(599)]
#[tokio::test]
async fn deployment_stores_the_first_qualifying_error_with_the_outage_ttl(
    #[case] status_code: i64,
) {
    let mut fixture = Fixture::new(model_deployment());
    fixture.args.outage_alert_ttl = 120;
    // One alert already meets this threshold, so an empty sender proves the first record never alerts.
    fixture.args.minor_outage_alert_threshold = 1;
    fixture
        .detector()
        .outage_alerts(Some(status_code), "dep-1")
        .await;
    let state = fixture.store.lock();
    assert_eq!(state.reads, ["dep-1".to_owned()]);
    assert_eq!(
        state.writes,
        [StoreWrite::Deployment {
            key: "dep-1".to_owned(),
            record: deployment_record(vec![status_code], NONE_SENT),
            ttl_seconds: Some(120),
        }]
    );
    drop(state);
    assert!(fixture.sender.sent().is_empty());
}

#[tokio::test]
async fn deployment_stores_later_updates_without_a_ttl() {
    let fixture = Fixture::new(model_deployment());
    fixture.store.lock().deployment = Some(deployment_record(vec![500], NONE_SENT));
    fixture.detector().outage_alerts(Some(500), "dep-1").await;
    let state = fixture.store.lock();
    assert_eq!(
        state.writes,
        [StoreWrite::Deployment {
            key: "dep-1".to_owned(),
            record: deployment_record(vec![500, 500], NONE_SENT),
            ttl_seconds: None,
        }]
    );
    drop(state);
    assert!(fixture.sender.sent().is_empty());
}

#[tokio::test]
async fn deployment_updates_the_last_check_time() {
    let fixture = Fixture::new(model_deployment());
    let mut previous_record = deployment_record(vec![500], NONE_SENT);
    previous_record.last_updated_at = 300.0_f64;
    fixture.store.lock().deployment = Some(previous_record);
    fixture.detector().outage_alerts(Some(500), "dep-1").await;
    let state = fixture.store.lock();
    assert_eq!(
        state.writes,
        [StoreWrite::Deployment {
            key: "dep-1".to_owned(),
            record: deployment_record(vec![500, 500], NONE_SENT),
            ttl_seconds: None,
        }]
    );
}

#[tokio::test]
async fn deployment_stops_the_alerts_list_at_the_max_size() {
    let mut fixture = Fixture::new(model_deployment());
    fixture.args.max_outage_alert_list_size = 3;
    fixture.store.lock().deployment = Some(deployment_record(vec![500, 501, 502], NONE_SENT));
    fixture.detector().outage_alerts(Some(503), "dep-1").await;
    let state = fixture.store.lock();
    assert_eq!(
        state.writes,
        [StoreWrite::Deployment {
            key: "dep-1".to_owned(),
            record: deployment_record(vec![500, 501, 502], NONE_SENT),
            ttl_seconds: None,
        }]
    );
    drop(state);
    assert!(fixture.sender.sent().is_empty());
}

#[tokio::test]
async fn deployment_sends_a_minor_alert_once_at_the_threshold() {
    let mut fixture = Fixture::new(model_deployment());
    fixture.args.minor_outage_alert_threshold = 2;
    fixture.store.lock().deployment = Some(deployment_record(vec![500], NONE_SENT));
    fixture.detector().outage_alerts(Some(500), "dep-1").await;
    fixture.detector().outage_alerts(Some(500), "dep-1").await;
    let sent = fixture.sender.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].level, AlertLevel::Medium);
    assert_eq!(sent[0].message, MODEL_MINOR_TWO);
    let state = fixture.store.lock();
    assert_eq!(state.writes.len(), 2);
    assert_eq!(
        state.writes[0],
        StoreWrite::Deployment {
            key: "dep-1".to_owned(),
            record: deployment_record(vec![500; 2], MINOR_SENT),
            ttl_seconds: None,
        }
    );
    assert_eq!(
        state.writes[1],
        StoreWrite::Deployment {
            key: "dep-1".to_owned(),
            record: deployment_record(vec![500; 3], MINOR_SENT),
            ttl_seconds: None,
        }
    );
}

#[tokio::test]
async fn deployment_sends_the_minor_alert_above_the_threshold_too() {
    let fixture = Fixture::new(model_deployment());
    fixture.store.lock().deployment = Some(deployment_record(vec![500; 6], NONE_SENT));
    fixture.detector().outage_alerts(Some(500), "dep-1").await;
    let sent = fixture.sender.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].level, AlertLevel::Medium);
    assert_eq!(sent[0].message, MODEL_MINOR_SEVEN);
}

#[tokio::test]
async fn deployment_sends_a_major_alert_once_at_the_major_threshold() {
    let mut fixture = Fixture::new(model_deployment());
    fixture.args.major_outage_alert_threshold = 4;
    fixture.store.lock().deployment = Some(deployment_record(vec![500; 3], MINOR_SENT));
    fixture.detector().outage_alerts(Some(500), "dep-1").await;
    fixture.detector().outage_alerts(Some(500), "dep-1").await;
    let sent = fixture.sender.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].level, AlertLevel::High);
    assert_eq!(sent[0].message, MODEL_MAJOR_FOUR);
    let state = fixture.store.lock();
    assert_eq!(state.writes.len(), 2);
    assert_eq!(
        state.writes[0],
        StoreWrite::Deployment {
            key: "dep-1".to_owned(),
            record: deployment_record(vec![500; 4], BOTH_SENT),
            ttl_seconds: None,
        }
    );
}

#[tokio::test]
async fn deployment_keeps_the_major_alert_until_the_minor_one_is_off() {
    let fixture = Fixture::new(model_deployment());
    fixture.store.lock().deployment = Some(deployment_record(vec![500; 9], NONE_SENT));
    fixture.detector().outage_alerts(Some(500), "dep-1").await;
    let sent = fixture.sender.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].level, AlertLevel::Medium);
    assert_eq!(sent[0].message, MODEL_MINOR_TEN);
    let state = fixture.store.lock();
    assert_eq!(
        state.writes,
        [StoreWrite::Deployment {
            key: "dep-1".to_owned(),
            record: deployment_record(vec![500; 10], MINOR_SENT),
            ttl_seconds: None,
        }]
    );
}

#[tokio::test]
async fn deployment_keeps_the_declared_provider_without_resolving() {
    let fixture = Fixture::new(model_deployment());
    fixture.detector().outage_alerts(Some(500), "dep-1").await;
    assert!(fixture.providers.models().is_empty());
    assert_eq!(
        fixture.api_base.calls(),
        [("gpt-4o".to_owned(), "dep-1".to_owned())]
    );
}

#[tokio::test]
async fn deployment_falls_back_to_get_llm_provider_when_no_provider_is_declared() {
    let mut deployment = model_deployment();
    deployment.model = "gpt-4o-mini".to_owned();
    deployment.custom_llm_provider = None;
    let mut fixture = Fixture::new(deployment);
    fixture.providers.result = Ok(ProviderResolution {
        model: "gpt-4o".to_owned(),
        provider: "openai".to_owned(),
    });
    fixture.store.lock().deployment = Some(deployment_record(vec![500; 4], NONE_SENT));
    fixture.detector().outage_alerts(Some(500), "dep-1").await;
    assert_eq!(fixture.providers.models(), ["gpt-4o-mini".to_owned()]);
    assert_eq!(
        fixture.api_base.calls(),
        [("gpt-4o".to_owned(), "dep-1".to_owned())]
    );
    let sent = fixture.sender.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].message, MODEL_MINOR_FIVE);
}

#[tokio::test]
async fn deployment_uses_an_empty_provider_and_the_unchanged_model_when_resolution_fails() {
    let mut deployment = model_deployment();
    deployment.model = "gpt-4o-mini".to_owned();
    deployment.custom_llm_provider = None;
    let mut fixture = Fixture::new(deployment);
    fixture.providers.result = Err(FakeFailure);
    fixture.store.lock().deployment = Some(deployment_record(vec![500; 4], NONE_SENT));
    fixture.detector().outage_alerts(Some(500), "dep-1").await;
    assert_eq!(fixture.providers.models(), ["gpt-4o-mini".to_owned()]);
    assert_eq!(
        fixture.api_base.calls(),
        [("gpt-4o-mini".to_owned(), "dep-1".to_owned())]
    );
    let sent = fixture.sender.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].message, MODEL_UNRESOLVED_FIVE);
}

#[tokio::test]
async fn deployment_drops_a_cache_read_error() {
    let fixture = Fixture::new(model_deployment());
    fixture.store.lock().fail_read = true;
    fixture.detector().outage_alerts(Some(500), "dep-1").await;
    let state = fixture.store.lock();
    assert_eq!(state.reads, ["dep-1".to_owned()]);
    assert!(state.writes.is_empty());
    assert!(state.write_attempts.is_empty());
    drop(state);
    assert!(fixture.deployments.lookups().is_empty());
    assert!(fixture.sender.sent().is_empty());
}

#[tokio::test]
async fn deployment_drops_a_cache_write_error() {
    let fixture = Fixture::new(model_deployment());
    fixture.store.lock().fail_write = true;
    fixture.detector().outage_alerts(Some(500), "dep-1").await;
    let state = fixture.store.lock();
    assert_eq!(state.write_attempts.len(), 1);
    assert!(state.writes.is_empty());
    drop(state);
    assert!(fixture.sender.sent().is_empty());
}

#[tokio::test]
async fn deployment_drops_a_deployment_lookup_error() {
    let mut fixture = Fixture::new(model_deployment());
    fixture.deployments.fail = true;
    fixture.detector().outage_alerts(Some(500), "dep-1").await;
    let state = fixture.store.lock();
    assert!(state.write_attempts.is_empty());
    drop(state);
    assert!(fixture.sender.sent().is_empty());
}

#[tokio::test]
async fn deployment_drops_an_api_base_error_before_storing() {
    let mut fixture = Fixture::new(model_deployment());
    fixture.api_base.result = Err(FakeFailure);
    fixture.detector().outage_alerts(Some(500), "dep-1").await;
    let state = fixture.store.lock();
    assert!(state.write_attempts.is_empty());
    drop(state);
    assert!(fixture.sender.sent().is_empty());
}

#[tokio::test]
async fn deployment_drops_a_sender_error_without_storing() {
    let mut fixture = Fixture::new(model_deployment());
    fixture.store.lock().deployment = Some(deployment_record(vec![500; 4], NONE_SENT));
    fixture.sender.fail = true;
    fixture.detector().outage_alerts(Some(500), "dep-1").await;
    assert!(fixture.sender.sent().is_empty());
    let state = fixture.store.lock();
    assert!(state.write_attempts.is_empty());
    drop(state);
}
