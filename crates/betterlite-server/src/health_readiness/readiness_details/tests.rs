#![cfg(test)]

use std::sync::Arc;

use axum::body::to_bytes;
use axum::http::StatusCode;
use betterlite_config::sections::aiohttp_transport_toggle::Settings as AiohttpSettings;
use betterlite_config::sections::env_credential_login::Settings as EnvLoginSettings;
use betterlite_config::sections::health_readiness::Settings as HealthSettings;
use betterlite_db::database_availability::policy::AllowRequestsOnOutage;
use betterlite_db::database_faults::ErrorType;
use rstest::rstest;
use serde::Deserialize;

use super::{CacheReport, respond};
use crate::health_readiness::db_readiness::DbReadiness;
use crate::health_readiness::tests::support::{
    FakeProbe, FakeSources, FakeStall, ProbeAnswer, WorkerCensus, db_readiness, parse,
};

#[derive(Debug, Deserialize)]
#[serde(transparent)]
struct WireBool(bool);

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
enum CacheJson {
    Plain(String),
    Semantic {
        #[serde(rename = "type")]
        cache_type: String,
        index_info: String,
    },
}

#[derive(Debug, Deserialize)]
struct DetailsBody {
    status: String,
    db: String,
    cache: Option<CacheJson>,
    litellm_version: String,
    success_callbacks: Vec<String>,
    use_aiohttp_transport: WireBool,
    log_level: String,
    is_detailed_debug: WireBool,
    show_no_redis_warning: WireBool,
    #[serde(rename = "show_env_credential_login_warning")]
    env_login_warning: WireBool,
}

#[derive(Debug, Deserialize)]
struct DetailBody {
    detail: String,
}

struct Setup {
    db: DbReadiness,
    sources: Arc<FakeSources>,
}

fn setup_with_stall(
    probe: FakeProbe,
    availability: AllowRequestsOnOutage,
    sources: FakeSources,
    stalled: bool,
) -> Setup {
    let probe = Arc::new(probe);
    let stall = Arc::new(FakeStall { stalled });
    let db = db_readiness(&probe, &stall, availability, &HealthSettings::default());
    Setup {
        db,
        sources: Arc::new(sources),
    }
}

fn setup(probe: FakeProbe, availability: AllowRequestsOnOutage, sources: FakeSources) -> Setup {
    setup_with_stall(probe, availability, sources, false)
}

async fn call(
    setup: &Setup,
    env_login: &EnvLoginSettings,
    aiohttp: &AiohttpSettings,
) -> (StatusCode, Vec<u8>) {
    let response = respond(&setup.db, &*setup.sources, env_login, aiohttp).await;
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body is readable");
    (status, body.to_vec())
}

async fn call_plain(setup: &Setup) -> (StatusCode, Vec<u8>) {
    call(
        setup,
        &EnvLoginSettings::default(),
        &AiohttpSettings::default(),
    )
    .await
}

#[tokio::test]
async fn details_body_reports_every_field() {
    let mut sources = FakeSources::default();
    sources.callbacks = vec!["langfuse".to_owned()];
    sources.cache = Ok(Some(CacheReport::Semantic {
        cache_type: "redis_semantic".to_owned(),
        index_info: "{\"num_docs\": 1}".to_owned(),
    }));
    sources.level = "DEBUG".to_owned();
    sources.version = "1.2.3".to_owned();
    let mut probe = FakeProbe::healthy();
    probe.census = WorkerCensus::Workers(2);
    let setup = setup(probe, AllowRequestsOnOutage::Unsupported, sources);
    let (status, body) = call_plain(&setup).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: DetailsBody = parse(&body);
    assert_eq!(parsed.status, "healthy");
    assert_eq!(parsed.db, "connected");
    assert_eq!(
        parsed.cache,
        Some(CacheJson::Semantic {
            cache_type: "redis_semantic".to_owned(),
            index_info: "{\"num_docs\": 1}".to_owned(),
        })
    );
    assert_eq!(parsed.litellm_version, "1.2.3");
    assert_eq!(parsed.success_callbacks, vec!["langfuse".to_owned()]);
    assert!(parsed.use_aiohttp_transport.0);
    assert_eq!(parsed.log_level, "DEBUG");
    assert!(parsed.is_detailed_debug.0);
    assert!(parsed.show_no_redis_warning.0);
    assert!(parsed.env_login_warning.0);
}

#[tokio::test]
async fn details_cache_is_null_without_a_cache() {
    let setup = setup(
        FakeProbe::healthy(),
        AllowRequestsOnOutage::Unsupported,
        FakeSources::default(),
    );
    let (status, body) = call_plain(&setup).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: DetailsBody = parse(&body);
    assert!(parsed.cache.is_none());
}

#[tokio::test]
async fn details_cache_keeps_a_plain_type() {
    let mut sources = FakeSources::default();
    sources.cache = Ok(Some(CacheReport::Plain("local".to_owned())));
    let setup = setup(
        FakeProbe::healthy(),
        AllowRequestsOnOutage::Unsupported,
        sources,
    );
    let (_, body) = call_plain(&setup).await;
    let parsed: DetailsBody = parse(&body);
    assert_eq!(parsed.cache, Some(CacheJson::Plain("local".to_owned())));
}

#[tokio::test]
async fn details_aiohttp_transport_is_false_when_disabled() {
    let setup = setup(
        FakeProbe::healthy(),
        AllowRequestsOnOutage::Unsupported,
        FakeSources::default(),
    );
    let aiohttp = AiohttpSettings {
        disable_aiohttp_transport: true,
    };
    let (_, body) = call(&setup, &EnvLoginSettings::default(), &aiohttp).await;
    let parsed: DetailsBody = parse(&body);
    assert!(!parsed.use_aiohttp_transport.0);
}

#[tokio::test]
async fn details_database_failure_flips_to_503() {
    let setup = setup(
        FakeProbe::scripted(vec![ProbeAnswer::Fail(ErrorType::Data)]),
        AllowRequestsOnOutage::Unsupported,
        FakeSources::default(),
    );
    let (status, body) = call_plain(&setup).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let parsed: DetailsBody = parse(&body);
    assert_eq!(parsed.db, "disconnected");
}

#[tokio::test]
async fn details_database_failure_keeps_200_when_requests_are_allowed() {
    let setup = setup(
        FakeProbe::scripted(vec![ProbeAnswer::Fail(ErrorType::Data)]),
        AllowRequestsOnOutage::Boolean(true),
        FakeSources::default(),
    );
    let (status, body) = call_plain(&setup).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: DetailsBody = parse(&body);
    assert_eq!(parsed.db, "disconnected");
}

#[tokio::test]
async fn details_without_a_database_stays_200_and_reports_not_connected() {
    let setup = setup(
        FakeProbe::unconfigured(),
        AllowRequestsOnOutage::Unsupported,
        FakeSources::default(),
    );
    let (status, body) = call_plain(&setup).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: DetailsBody = parse(&body);
    assert_eq!(parsed.db, "Not connected");
}

#[tokio::test]
async fn details_stalled_database_flips_to_503() {
    let setup = setup_with_stall(
        FakeProbe::healthy(),
        AllowRequestsOnOutage::Unsupported,
        FakeSources::default(),
        true,
    );
    let (status, body) = call_plain(&setup).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let parsed: DetailsBody = parse(&body);
    assert_eq!(parsed.db, "stalled");
}

#[tokio::test]
async fn details_source_failure_is_503_service_unhealthy() {
    let mut sources = FakeSources::default();
    sources.cache = Err("cache exploded".to_owned());
    let setup = setup(
        FakeProbe::healthy(),
        AllowRequestsOnOutage::Unsupported,
        sources,
    );
    let (status, body) = call_plain(&setup).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let parsed: DetailBody = parse(&body);
    assert_eq!(parsed.detail, "Service Unhealthy (cache exploded)");
}

#[rstest]
#[case::info("INFO", false)]
#[case::debug("DEBUG", true)]
#[case::notset("NOTSET", true)]
#[case::warning("WARNING", false)]
#[tokio::test]
async fn details_is_detailed_debug_at_debug_or_below(#[case] level: &str, #[case] expected: bool) {
    let mut sources = FakeSources::default();
    sources.level = level.to_owned();
    let setup = setup(
        FakeProbe::healthy(),
        AllowRequestsOnOutage::Unsupported,
        sources,
    );
    let (_, body) = call_plain(&setup).await;
    let parsed: DetailsBody = parse(&body);
    assert_eq!(parsed.log_level, level);
    assert_eq!(parsed.is_detailed_debug.0, expected);
}

#[tokio::test]
async fn details_env_login_warning_is_off_when_env_login_is_disabled() {
    let setup = setup(
        FakeProbe::healthy(),
        AllowRequestsOnOutage::Unsupported,
        FakeSources::default(),
    );
    let env_login = EnvLoginSettings {
        env_login_disabled: true,
        sso_blocks_password_login: false,
    };
    let (_, body) = call(&setup, &env_login, &AiohttpSettings::default()).await;
    let parsed: DetailsBody = parse(&body);
    assert!(!parsed.env_login_warning.0);
}

#[tokio::test]
async fn details_env_login_warning_is_off_when_sso_blocks_login_and_is_ready() {
    let mut sources = FakeSources::default();
    sources.sso = true;
    let setup = setup(
        FakeProbe::healthy(),
        AllowRequestsOnOutage::Unsupported,
        sources,
    );
    let env_login = EnvLoginSettings {
        env_login_disabled: false,
        sso_blocks_password_login: true,
    };
    let (_, body) = call(&setup, &env_login, &AiohttpSettings::default()).await;
    let parsed: DetailsBody = parse(&body);
    assert!(!parsed.env_login_warning.0);
}

#[tokio::test]
async fn details_env_login_warning_stays_on_when_sso_is_not_ready() {
    let setup = setup(
        FakeProbe::healthy(),
        AllowRequestsOnOutage::Unsupported,
        FakeSources::default(),
    );
    let env_login = EnvLoginSettings {
        env_login_disabled: false,
        sso_blocks_password_login: true,
    };
    let (_, body) = call(&setup, &env_login, &AiohttpSettings::default()).await;
    let parsed: DetailsBody = parse(&body);
    assert!(parsed.env_login_warning.0);
}
