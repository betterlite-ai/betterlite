use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_config::sections::pointfive_params::PointFiveInitParams;

use crate::exporter_options::health::HealthState;
use crate::finops::pointfive::logger::PointFiveLogger;
use crate::finops::pointfive::logger::core::PointFiveCore;
use crate::finops::pointfive::logger::flush::ping;

use super::support::{FakeHttp, Lapse, LapsingSource, Reply, client, deps_from, logged, logger};

const MISSING_KEY: &str = "pointfive logging requires an api key. Set POINTFIVE_API_KEY, or litellm_settings.pointfive_params.api_key in config.yaml";

#[tokio::test]
async fn a_missing_key_reports_unhealthy() {
    let http = FakeHttp::default();
    let available = Arc::new(AtomicBool::new(true));
    let source = Arc::new(LapsingSource {
        client: client(&http, 1),
        available: Arc::clone(&available),
        lapse: Lapse::MissingKey,
    });
    let logger = PointFiveLogger::new(
        PointFiveInitParams::default(),
        deps_from(source, None, false),
    )
    .expect("the key resolves at startup");
    available.store(false, Ordering::SeqCst);
    let status = logger.async_health_check().await;
    assert_eq!(status.status, HealthState::Unhealthy);
    assert_eq!(status.error_message.as_deref(), Some(MISSING_KEY));
}

#[tokio::test]
async fn a_successful_ping_reports_healthy() {
    let http = FakeHttp::scripted(vec![Reply::answer(200)]);
    let logger = logger(PointFiveInitParams::default(), client(&http, 1));
    let status = logger.async_health_check().await;
    assert_eq!(status.status, HealthState::Healthy);
    assert!(status.error_message.is_none());
    assert_eq!(http.pings().len(), 1);
}

#[tokio::test]
async fn a_rejected_ping_reports_unhealthy() {
    let http = FakeHttp::scripted(vec![Reply::answer(401)]);
    let logger = logger(PointFiveInitParams::default(), client(&http, 1));
    let status = logger.async_health_check().await;
    assert_eq!(status.status, HealthState::Unhealthy);
    assert_eq!(
        status.error_message.as_deref(),
        Some("pointfive api returned 401")
    );
}

#[tokio::test]
async fn a_secret_lookup_error_reports_unhealthy() {
    let http = FakeHttp::default();
    let available = Arc::new(AtomicBool::new(true));
    let source = Arc::new(LapsingSource {
        client: client(&http, 1),
        available: Arc::clone(&available),
        lapse: Lapse::Secret,
    });
    let logger = PointFiveLogger::new(
        PointFiveInitParams::default(),
        deps_from(source, None, false),
    )
    .expect("the client resolves at startup");
    available.store(false, Ordering::SeqCst);
    let status = logger.async_health_check().await;
    assert_eq!(status.status, HealthState::Unhealthy);
    assert!(
        status
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("the pointfive secret could not be read"))
    );
}

#[tokio::test]
async fn a_client_that_cannot_be_built_is_only_warned_about() {
    let http = FakeHttp::default();
    let available = Arc::new(AtomicBool::new(true));
    let source = Arc::new(LapsingSource {
        client: client(&http, 1),
        available: Arc::clone(&available),
        lapse: Lapse::Secret,
    });
    let core = PointFiveCore::new(PointFiveInitParams::default(), None, source, false);
    available.store(false, Ordering::SeqCst);
    let ((), text) = logged(ping(&core)).await;
    assert!(text.contains("liveness ping skipped"));
    assert!(text.contains("the pointfive secret could not be read"));
}
