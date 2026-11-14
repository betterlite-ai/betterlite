use betterlite_config::sections::alert_settings::types::AlertType;

use crate::outage_alerts::entry::FailureKind;

use super::support::{
    FakeFailure, Fixture, MINOR_SENT, NONE_SENT, StoreWrite, deployment_record, model_deployment,
    region_record, vertex_deployment,
};

#[tokio::test]
async fn entry_runs_the_deployment_detector_only_for_its_alert_type() {
    let fixture = Fixture::new(model_deployment());
    fixture
        .detector()
        .on_failure(
            &[AlertType::OutageAlerts],
            FailureKind::ApiError {
                status_code: Some(500),
            },
            "dep-1",
        )
        .await;
    assert_eq!(fixture.deployments.lookups(), ["dep-1".to_owned()]);
    assert_eq!(fixture.store.lock().reads, ["dep-1".to_owned()]);
}

#[tokio::test]
async fn entry_runs_the_region_detector_only_for_its_alert_type() {
    let fixture = Fixture::new(vertex_deployment());
    fixture
        .detector()
        .on_failure(
            &[AlertType::RegionOutageAlerts],
            FailureKind::ApiError {
                status_code: Some(500),
            },
            "dep-1",
        )
        .await;
    assert_eq!(fixture.deployments.lookups(), ["dep-1".to_owned()]);
    assert_eq!(fixture.store.lock().reads, ["vertex_aius-east1".to_owned()]);
}

#[tokio::test]
async fn entry_runs_both_detectors_in_order() {
    let fixture = Fixture::new(vertex_deployment());
    fixture
        .detector()
        .on_failure(
            &[AlertType::OutageAlerts, AlertType::RegionOutageAlerts],
            FailureKind::ApiError {
                status_code: Some(500),
            },
            "dep-1",
        )
        .await;
    let state = fixture.store.lock();
    assert_eq!(
        state.reads,
        ["dep-1".to_owned(), "vertex_aius-east1".to_owned()]
    );
    assert_eq!(state.writes.len(), 2);
    assert!(matches!(state.writes[0], StoreWrite::Deployment { .. }));
    assert!(matches!(state.writes[1], StoreWrite::Region { .. }));
    drop(state);
}

#[tokio::test]
async fn entry_forwards_the_status_code_to_the_deployment_detector() {
    let fixture = Fixture::new(model_deployment());
    fixture.store.lock().deployment = Some(deployment_record(vec![500; 4], NONE_SENT));
    fixture
        .detector()
        .on_failure(
            &[AlertType::OutageAlerts],
            FailureKind::ApiError {
                status_code: Some(408),
            },
            "dep-1",
        )
        .await;
    assert_eq!(
        fixture.store.lock().writes,
        [StoreWrite::Deployment {
            key: "dep-1".to_owned(),
            record: deployment_record(vec![500, 500, 500, 500, 408], MINOR_SENT),
            ttl_seconds: None,
        }]
    );
}

#[tokio::test]
async fn entry_forwards_the_status_code_to_the_region_detector() {
    let fixture = Fixture::new(vertex_deployment());
    fixture.store.lock().region = Some(region_record(
        "vertex_aius-east1",
        vec![500; 4],
        &["dep-0"],
        NONE_SENT,
    ));
    fixture
        .detector()
        .on_failure(
            &[AlertType::RegionOutageAlerts],
            FailureKind::ApiError {
                status_code: Some(408),
            },
            "dep-1",
        )
        .await;
    assert_eq!(
        fixture.store.lock().writes,
        [StoreWrite::Region {
            key: "vertex_aius-east1".to_owned(),
            record: region_record(
                "vertex_aius-east1",
                vec![500, 500, 500, 500, 408],
                &["dep-0", "dep-1"],
                MINOR_SENT,
            ),
            ttl_seconds: None,
        }]
    );
}

#[tokio::test]
async fn entry_ignores_a_failure_that_is_not_an_api_error() {
    let fixture = Fixture::new(vertex_deployment());
    fixture
        .detector()
        .on_failure(
            &[AlertType::OutageAlerts, AlertType::RegionOutageAlerts],
            FailureKind::Other,
            "dep-1",
        )
        .await;
    assert!(fixture.deployments.lookups().is_empty());
    assert!(fixture.store.lock().reads.is_empty());
}

#[tokio::test]
async fn entry_ignores_a_failure_with_no_alert_type_listed() {
    let fixture = Fixture::new(vertex_deployment());
    fixture
        .detector()
        .on_failure(
            &[],
            FailureKind::ApiError {
                status_code: Some(500),
            },
            "dep-1",
        )
        .await;
    assert!(fixture.deployments.lookups().is_empty());
    assert!(fixture.store.lock().reads.is_empty());
}

#[tokio::test]
async fn entry_drops_a_region_detector_error() {
    let mut deployment = vertex_deployment();
    deployment.custom_llm_provider = None;
    let mut fixture = Fixture::new(deployment);
    fixture.providers.result = Err(FakeFailure);
    fixture
        .detector()
        .on_failure(
            &[AlertType::RegionOutageAlerts],
            FailureKind::ApiError {
                status_code: Some(500),
            },
            "dep-1",
        )
        .await;
    assert_eq!(fixture.providers.models(), ["gemini-1.5-pro".to_owned()]);
    assert!(fixture.store.lock().reads.is_empty());
}
