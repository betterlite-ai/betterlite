use rstest::rstest;

use crate::alert_dispatch::AlertLevel;
use crate::outage_alerts::error::RegionOutageError;

use super::support::{
    BOTH_SENT, FakeFailure, Fixture, MINOR_SENT, NONE_SENT, StoreWrite, model_deployment,
    region_record, regional_deployment, vertex_deployment,
};

const REGION_MINOR_FIVE: &str = "\n\n\n*\u{26a0}\u{fe0f} Minor Service Outage*\n\n\n*Region Name:* `us-east1`\n*Provider:* `vertex_ai`\n\n\n*Errors:*\n\nAPI Errors: 5\n\n\n*Last Check:* `0.0s ago`\n\n\n";
const REGION_MINOR_SEVEN: &str = "\n\n\n*\u{26a0}\u{fe0f} Minor Service Outage*\n\n\n*Region Name:* `us-east1`\n*Provider:* `vertex_ai`\n\n\n*Errors:*\n\nAPI Errors: 7\n\n\n*Last Check:* `0.0s ago`\n\n\n";
const REGION_MINOR_TEN: &str = "\n\n\n*\u{26a0}\u{fe0f} Minor Service Outage*\n\n\n*Region Name:* `us-east1`\n*Provider:* `vertex_ai`\n\n\n*Errors:*\n\nAPI Errors: 10\n\n\n*Last Check:* `0.0s ago`\n\n\n";
const REGION_MINOR_ASIA_FIVE: &str = "\n\n\n*\u{26a0}\u{fe0f} Minor Service Outage*\n\n\n*Region Name:* `asia-east1`\n*Provider:* `vertex_ai`\n\n\n*Errors:*\n\nAPI Errors: 5\n\n\n*Last Check:* `0.0s ago`\n\n\n";
const REGION_MAJOR_TEN: &str = "\n\n\n*\u{26a0}\u{fe0f} Major Service Outage*\n\n\n*Region Name:* `us-east1`\n*Provider:* `vertex_ai`\n\n\n*Errors:*\n\nAPI Errors: 10\n\n\n*Last Check:* `0.0s ago`\n\n\n";

#[rstest]
#[case::timeout(408)]
#[case::server_error(500)]
#[case::above_server_error(599)]
#[tokio::test]
async fn region_stores_the_first_qualifying_error_with_the_region_ttl(#[case] status_code: i64) {
    let fixture = Fixture::new(vertex_deployment());
    fixture
        .detector()
        .region_outage_alerts(Some(status_code), "dep-1")
        .await
        .unwrap();
    let state = fixture.store.lock();
    assert_eq!(state.reads, ["vertex_aius-east1".to_owned()]);
    assert_eq!(
        state.writes,
        [StoreWrite::Region {
            key: "vertex_aius-east1".to_owned(),
            record: region_record(
                "vertex_aius-east1",
                vec![status_code],
                &["dep-1"],
                NONE_SENT,
            ),
            ttl_seconds: Some(60),
        }]
    );
    drop(state);
    assert!(fixture.providers.models().is_empty());
    assert!(fixture.api_base.calls().is_empty());
    assert!(fixture.sender.sent().is_empty());
}

#[rstest]
#[case::missing_status(None)]
#[case::unauthorized(Some(401))]
#[case::rate_limited(Some(429))]
#[case::client_error(Some(499))]
#[tokio::test]
async fn region_ignores_a_missing_or_non_qualifying_status_after_the_cache_read(
    #[case] status_code: Option<i64>,
) {
    let fixture = Fixture::new(vertex_deployment());
    fixture
        .detector()
        .region_outage_alerts(status_code, "dep-1")
        .await
        .unwrap();
    let state = fixture.store.lock();
    assert_eq!(state.reads, ["vertex_aius-east1".to_owned()]);
    assert!(state.writes.is_empty());
    assert!(state.write_attempts.is_empty());
    drop(state);
    assert!(fixture.sender.sent().is_empty());
}

#[tokio::test]
async fn region_takes_the_region_name_before_the_model_region() {
    let fixture_deployment = regional_deployment(
        "vertex_ai",
        Some("asia-east1"),
        Some("us-east1"),
        None,
        None,
    );
    let mut fixture = Fixture::new(fixture_deployment);
    fixture.store.lock().region = Some(region_record(
        "vertex_aiasia-east1",
        vec![500; 4],
        &["dep-0"],
        NONE_SENT,
    ));
    fixture
        .detector()
        .region_outage_alerts(Some(500), "dep-1")
        .await
        .unwrap();
    let sent = fixture.sender.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].level, AlertLevel::Medium);
    assert_eq!(sent[0].message, REGION_MINOR_ASIA_FIVE);
    let state = fixture.store.lock();
    assert_eq!(
        state.writes,
        [StoreWrite::Region {
            key: "vertex_aiasia-east1".to_owned(),
            record: region_record(
                "vertex_aiasia-east1",
                vec![500; 5],
                &["dep-0", "dep-1"],
                MINOR_SENT,
            ),
            ttl_seconds: None,
        }]
    );
}

#[tokio::test]
async fn region_falls_back_to_the_vertex_location() {
    let fixture = Fixture::new(regional_deployment(
        "vertex_ai",
        None,
        Some("us-east1"),
        None,
        None,
    ));
    fixture
        .detector()
        .region_outage_alerts(Some(500), "dep-1")
        .await
        .unwrap();
    assert_eq!(fixture.store.lock().reads, ["vertex_aius-east1".to_owned()]);
}

#[tokio::test]
async fn region_falls_back_to_the_bedrock_region() {
    let fixture = Fixture::new(regional_deployment(
        "bedrock",
        None,
        None,
        Some("us-west-2"),
        None,
    ));
    fixture
        .detector()
        .region_outage_alerts(Some(500), "dep-1")
        .await
        .unwrap();
    let state = fixture.store.lock();
    assert_eq!(state.reads, ["bedrockus-west-2".to_owned()]);
    assert_eq!(
        state.writes,
        [StoreWrite::Region {
            key: "bedrockus-west-2".to_owned(),
            record: region_record("bedrockus-west-2", vec![500], &["dep-1"], NONE_SENT),
            ttl_seconds: Some(60),
        }]
    );
}

#[tokio::test]
async fn region_falls_back_to_the_watsonx_region() {
    let fixture = Fixture::new(regional_deployment(
        "watsonx",
        None,
        None,
        None,
        Some("eu-de"),
    ));
    fixture
        .detector()
        .region_outage_alerts(Some(500), "dep-1")
        .await
        .unwrap();
    assert_eq!(fixture.store.lock().reads, ["watsonxeu-de".to_owned()]);
}

#[tokio::test]
async fn region_does_nothing_when_no_region_is_found() {
    let fixture = Fixture::new(model_deployment());
    fixture
        .detector()
        .region_outage_alerts(Some(500), "dep-1")
        .await
        .unwrap();
    assert_eq!(fixture.deployments.lookups(), ["dep-1".to_owned()]);
    let state = fixture.store.lock();
    assert!(state.reads.is_empty());
    assert!(state.write_attempts.is_empty());
    drop(state);
    assert!(fixture.sender.sent().is_empty());
}

#[tokio::test]
async fn region_ignores_a_missing_router_lookup() {
    let fixture = Fixture::new(vertex_deployment());
    fixture
        .detector_without_router()
        .region_outage_alerts(Some(500), "dep-1")
        .await
        .unwrap();
    assert!(fixture.store.lock().reads.is_empty());
    assert!(fixture.deployments.lookups().is_empty());
}

#[tokio::test]
async fn region_ignores_an_unknown_deployment() {
    let mut fixture = Fixture::new(vertex_deployment());
    fixture.deployments.found = None;
    fixture
        .detector()
        .region_outage_alerts(Some(500), "missing")
        .await
        .unwrap();
    assert_eq!(fixture.deployments.lookups(), ["missing".to_owned()]);
    assert!(fixture.store.lock().reads.is_empty());
}

#[tokio::test]
async fn region_updates_the_last_check_time() {
    let mut fixture = Fixture::new(vertex_deployment());
    let mut previous_record = region_record("vertex_aius-east1", vec![500], &["dep-1"], NONE_SENT);
    previous_record.last_updated_at = 300.0;
    fixture.store.lock().region = Some(previous_record);
    fixture
        .detector()
        .region_outage_alerts(Some(500), "dep-1")
        .await
        .unwrap();
    assert_eq!(
        fixture.store.lock().writes,
        [StoreWrite::Region {
            key: "vertex_aius-east1".to_owned(),
            record: region_record("vertex_aius-east1", vec![500; 2], &["dep-1"], NONE_SENT),
            ttl_seconds: None,
        }]
    );
}

#[tokio::test]
async fn region_never_alerts_while_only_one_deployment_is_recorded() {
    let mut fixture = Fixture::new(vertex_deployment());
    fixture.store.lock().region = Some(region_record(
        "vertex_aius-east1",
        vec![500; 9],
        &["dep-1"],
        NONE_SENT,
    ));
    fixture
        .detector()
        .region_outage_alerts(Some(500), "dep-1")
        .await
        .unwrap();
    assert!(fixture.sender.sent().is_empty());
    assert_eq!(
        fixture.store.lock().writes,
        [StoreWrite::Region {
            key: "vertex_aius-east1".to_owned(),
            record: region_record("vertex_aius-east1", vec![500; 10], &["dep-1"], NONE_SENT),
            ttl_seconds: None,
        }]
    );
}

#[tokio::test]
async fn region_sends_a_minor_alert_when_two_deployments_and_the_threshold_qualify() {
    let mut fixture = Fixture::new(vertex_deployment());
    fixture.store.lock().region = Some(region_record(
        "vertex_aius-east1",
        vec![500; 4],
        &["dep-0"],
        NONE_SENT,
    ));
    fixture
        .detector()
        .region_outage_alerts(Some(500), "dep-1")
        .await
        .unwrap();
    fixture
        .detector()
        .region_outage_alerts(Some(500), "dep-1")
        .await
        .unwrap();
    let sent = fixture.sender.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].level, AlertLevel::Medium);
    assert_eq!(sent[0].message, REGION_MINOR_FIVE);
    let state = fixture.store.lock();
    assert_eq!(state.writes.len(), 2);
    assert_eq!(
        state.writes[0],
        StoreWrite::Region {
            key: "vertex_aius-east1".to_owned(),
            record: region_record(
                "vertex_aius-east1",
                vec![500; 5],
                &["dep-0", "dep-1"],
                MINOR_SENT,
            ),
            ttl_seconds: None,
        }
    );
}

#[tokio::test]
async fn region_sends_the_minor_alert_above_the_threshold_too() {
    let mut fixture = Fixture::new(vertex_deployment());
    fixture.store.lock().region = Some(region_record(
        "vertex_aius-east1",
        vec![500; 6],
        &["dep-0"],
        NONE_SENT,
    ));
    fixture
        .detector()
        .region_outage_alerts(Some(500), "dep-1")
        .await
        .unwrap();
    let sent = fixture.sender.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].message, REGION_MINOR_SEVEN);
}

#[tokio::test]
async fn region_sends_a_major_alert_once_the_minor_one_was_sent() {
    let mut fixture = Fixture::new(vertex_deployment());
    fixture.store.lock().region = Some(region_record(
        "vertex_aius-east1",
        vec![500; 9],
        &["dep-0"],
        MINOR_SENT,
    ));
    fixture
        .detector()
        .region_outage_alerts(Some(500), "dep-1")
        .await
        .unwrap();
    fixture
        .detector()
        .region_outage_alerts(Some(500), "dep-1")
        .await
        .unwrap();
    let sent = fixture.sender.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].level, AlertLevel::High);
    assert_eq!(sent[0].message, REGION_MAJOR_TEN);
    assert_eq!(
        fixture.store.lock().writes[0],
        StoreWrite::Region {
            key: "vertex_aius-east1".to_owned(),
            record: region_record(
                "vertex_aius-east1",
                vec![500; 10],
                &["dep-0", "dep-1"],
                BOTH_SENT,
            ),
            ttl_seconds: None,
        }
    );
}

#[tokio::test]
async fn region_keeps_the_major_alert_until_the_minor_one_is_off() {
    let mut fixture = Fixture::new(vertex_deployment());
    fixture.store.lock().region = Some(region_record(
        "vertex_aius-east1",
        vec![500; 9],
        &["dep-0"],
        NONE_SENT,
    ));
    fixture
        .detector()
        .region_outage_alerts(Some(500), "dep-1")
        .await
        .unwrap();
    let sent = fixture.sender.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].level, AlertLevel::Medium);
    assert_eq!(sent[0].message, REGION_MINOR_TEN);
}

#[tokio::test]
async fn region_stops_the_alerts_list_at_the_max_size() {
    let mut fixture = Fixture::new(vertex_deployment());
    fixture.args.max_outage_alert_list_size = 3;
    fixture.store.lock().region = Some(region_record(
        "vertex_aius-east1",
        vec![500, 501, 502],
        &["dep-0"],
        BOTH_SENT,
    ));
    fixture
        .detector()
        .region_outage_alerts(Some(503), "dep-1")
        .await
        .unwrap();
    assert_eq!(
        fixture.store.lock().writes,
        [StoreWrite::Region {
            key: "vertex_aius-east1".to_owned(),
            record: region_record(
                "vertex_aius-east1",
                vec![500, 501, 502],
                &["dep-0", "dep-1"],
                BOTH_SENT,
            ),
            ttl_seconds: None,
        }]
    );
}

#[tokio::test]
async fn region_returns_a_deployment_lookup_error() {
    let mut fixture = Fixture::new(vertex_deployment());
    fixture.deployments.fail = true;
    let result = fixture
        .detector()
        .region_outage_alerts(Some(500), "dep-1")
        .await;
    assert!(matches!(
        result,
        Err(RegionOutageError::Deployments(FakeFailure))
    ));
    assert!(fixture.store.lock().reads.is_empty());
}

#[tokio::test]
async fn region_returns_a_provider_resolution_error() {
    let mut deployment = vertex_deployment();
    deployment.custom_llm_provider = None;
    let mut fixture = Fixture::new(deployment);
    fixture.providers.result = Err(FakeFailure);
    let result = fixture
        .detector()
        .region_outage_alerts(Some(500), "dep-1")
        .await;
    assert!(matches!(
        result,
        Err(RegionOutageError::Providers(FakeFailure))
    ));
    assert_eq!(fixture.providers.models(), ["gemini-1.5-pro".to_owned()]);
    assert!(fixture.store.lock().reads.is_empty());
}

#[tokio::test]
async fn region_returns_a_cache_read_error() {
    let mut fixture = Fixture::new(vertex_deployment());
    fixture.store.lock().fail_read = true;
    let result = fixture
        .detector()
        .region_outage_alerts(Some(500), "dep-1")
        .await;
    assert!(matches!(result, Err(RegionOutageError::Read(FakeFailure))));
    assert_eq!(fixture.store.lock().reads, ["vertex_aius-east1".to_owned()]);
}

#[tokio::test]
async fn region_returns_a_cache_write_error() {
    let mut fixture = Fixture::new(vertex_deployment());
    fixture.store.lock().fail_write = true;
    let result = fixture
        .detector()
        .region_outage_alerts(Some(500), "dep-1")
        .await;
    assert!(matches!(result, Err(RegionOutageError::Write(FakeFailure))));
    assert_eq!(fixture.store.lock().write_attempts.len(), 1);
}

#[tokio::test]
async fn region_returns_the_error_of_the_final_cache_write() {
    let mut fixture = Fixture::new(vertex_deployment());
    fixture.store.lock().region = Some(region_record(
        "vertex_aius-east1",
        vec![500; 4],
        &["dep-0"],
        NONE_SENT,
    ));
    fixture.store.lock().fail_write = true;
    let result = fixture
        .detector()
        .region_outage_alerts(Some(500), "dep-1")
        .await;
    assert!(matches!(result, Err(RegionOutageError::Write(FakeFailure))));
    assert_eq!(fixture.sender.sent().len(), 1);
    assert_eq!(fixture.store.lock().write_attempts.len(), 1);
}

#[tokio::test]
async fn region_returns_a_sender_error() {
    let mut fixture = Fixture::new(vertex_deployment());
    fixture.store.lock().region = Some(region_record(
        "vertex_aius-east1",
        vec![500; 4],
        &["dep-0"],
        NONE_SENT,
    ));
    fixture.sender.fail = true;
    let result = fixture
        .detector()
        .region_outage_alerts(Some(500), "dep-1")
        .await;
    assert!(matches!(result, Err(RegionOutageError::Send(FakeFailure))));
    assert!(fixture.store.lock().write_attempts.is_empty());
}
