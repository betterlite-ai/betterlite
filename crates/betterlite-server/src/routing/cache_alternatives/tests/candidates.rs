use betterlite_models::decisions::Json;
use betterlite_router::router_predicates::JsonMap;

use crate::routing::cache_alternatives::candidates::{available, candidate};
use crate::routing::cache_alternatives::error::CacheAlternativesError;
use crate::routing::cache_alternatives::tests::support::{
    CALLER, CHEAP_ID, Fixture, caller, cheap_deployment, json, strong_deployment,
};

fn no_kwargs() -> JsonMap {
    JsonMap::new()
}

#[tokio::test]
async fn candidate_requires_exactly_one_deployment() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.deployments.insert(
        "twin".to_owned(),
        vec![cheap_deployment(30_000), strong_deployment(30_000)],
    );
    host.deployments.insert("none".to_owned(), Vec::new());
    let services = host.prediction_services();
    let found = candidate(&host, &services, "SIMPLE", "cheap", &no_kwargs())
        .unwrap()
        .expect("one plain deployment qualifies");
    assert_eq!(found.model, "cheap");
    assert_eq!(found.tier, "SIMPLE");
    assert_eq!(found.deployment.id(), Some(CHEAP_ID));
    assert!(
        candidate(&host, &services, "SIMPLE", "twin", &no_kwargs())
            .unwrap()
            .is_none()
    );
    assert!(
        candidate(&host, &services, "SIMPLE", "none", &no_kwargs())
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn candidate_refuses_a_blocked_deployment() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let mut host = fixture.host(&context);
    let mut spec = cheap_deployment(30_000);
    spec.model_info
        .insert("blocked".to_owned(), Json::Bool(true));
    host.deployments.insert("cheap".to_owned(), vec![spec]);
    let services = host.prediction_services();
    assert!(
        candidate(&host, &services, "SIMPLE", "cheap", &no_kwargs())
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn candidate_refuses_a_deployment_without_an_id() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let mut host = fixture.host(&context);
    let mut spec = cheap_deployment(30_000);
    assert!(spec.model_info.remove("id").is_some());
    host.deployments.insert("cheap".to_owned(), vec![spec]);
    let services = host.prediction_services();
    assert!(
        candidate(&host, &services, "SIMPLE", "cheap", &no_kwargs())
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn candidate_refuses_an_unsupported_prediction_target() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let mut host = fixture.host(&context);
    let mut spec = cheap_deployment(30_000);
    spec.litellm_params
        .insert("temperature".to_owned(), json("0.1"));
    host.deployments.insert("cheap".to_owned(), vec![spec]);
    let services = host.prediction_services();
    assert!(
        candidate(&host, &services, "SIMPLE", "cheap", &no_kwargs())
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn candidate_reports_a_deployment_lookup_error() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.missing.push("cheap".to_owned());
    let services = host.prediction_services();
    let error = candidate(&host, &services, "SIMPLE", "cheap", &no_kwargs()).unwrap_err();
    assert!(matches!(error, CacheAlternativesError::Deployments { .. }));
}

#[tokio::test]
async fn available_requires_the_candidate_deployment_to_be_healthy() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let host = fixture.host(&context);
    let services = host.prediction_services();
    let found = candidate(&host, &services, "SIMPLE", "cheap", &no_kwargs())
        .unwrap()
        .expect("one plain deployment qualifies");
    assert!(
        available(&host, &found, &caller(CALLER), &no_kwargs(), None)
            .await
            .unwrap()
    );
    assert_eq!(host.authorization_calls(), vec!["cheap".to_owned()]);
    let mut other = fixture.host(&context);
    other
        .deployments
        .insert("cheap".to_owned(), vec![strong_deployment(30_000)]);
    assert!(
        !available(&other, &found, &caller(CALLER), &no_kwargs(), None)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn available_treats_an_authorization_error_as_unavailable() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.authorized.retain(|name| name.as_str() != "cheap");
    let services = host.prediction_services();
    let found = candidate(&host, &services, "SIMPLE", "cheap", &no_kwargs())
        .unwrap()
        .expect("one plain deployment qualifies");
    let error = available(&host, &found, &caller(CALLER), &no_kwargs(), None)
        .await
        .unwrap_err();
    assert!(matches!(error, CacheAlternativesError::Access { .. }));
}

#[tokio::test]
async fn available_treats_a_health_lookup_error_as_unavailable() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.unhealthy.push("cheap".to_owned());
    let services = host.prediction_services();
    let found = candidate(&host, &services, "SIMPLE", "cheap", &no_kwargs())
        .unwrap()
        .expect("one plain deployment qualifies");
    let error = available(&host, &found, &caller(CALLER), &no_kwargs(), None)
        .await
        .unwrap_err();
    assert!(matches!(error, CacheAlternativesError::Healthy { .. }));
}

#[tokio::test]
async fn available_checks_authorization_before_health() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.authorized.retain(|name| name.as_str() != "cheap");
    host.unhealthy.push("cheap".to_owned());
    let services = host.prediction_services();
    let found = candidate(&host, &services, "SIMPLE", "cheap", &no_kwargs())
        .unwrap()
        .expect("one plain deployment qualifies");
    let error = available(&host, &found, &caller(CALLER), &no_kwargs(), None)
        .await
        .unwrap_err();
    assert!(matches!(error, CacheAlternativesError::Access { .. }));
}
