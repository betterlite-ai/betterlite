#![cfg(test)]

use std::sync::PoisonError;

use rstest::rstest;

use crate::otel_provider_assembly::proxy_registry::{
    OtelRegistration, ProxyRegistry, ServiceCallback, register_on_proxy,
};

fn logger(name: &str) -> OtelRegistration {
    OtelRegistration {
        callback_name: Some(name.to_owned()),
    }
}

fn callbacks(registry: &ProxyRegistry) -> Vec<ServiceCallback> {
    registry
        .service_callbacks
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

fn proxy_logger(registry: &ProxyRegistry) -> Option<OtelRegistration> {
    registry
        .open_telemetry_logger
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

#[test]
fn the_logger_joins_an_empty_callback_list_and_takes_the_empty_proxy_slot() {
    let registry = ProxyRegistry::default();
    register_on_proxy(&registry, &logger("first"));
    assert_eq!(
        callbacks(&registry),
        vec![ServiceCallback::OpenTelemetry(logger("first"))]
    );
    assert_eq!(proxy_logger(&registry), Some(logger("first")));
}

#[test]
fn the_logger_is_appended_after_unrelated_service_callbacks() {
    let registry = ProxyRegistry::default();
    registry
        .service_callbacks
        .lock()
        .unwrap()
        .push(ServiceCallback::Named("prometheus".to_owned()));
    register_on_proxy(&registry, &logger("first"));
    assert_eq!(
        callbacks(&registry),
        vec![
            ServiceCallback::Named("prometheus".to_owned()),
            ServiceCallback::OpenTelemetry(logger("first")),
        ]
    );
}

#[rstest]
#[case::the_otel_name(ServiceCallback::Named("otel".to_owned()))]
#[case::another_logger(ServiceCallback::OpenTelemetry(logger("earlier")))]
fn nothing_is_appended_when_an_otel_callback_is_already_registered(
    #[case] existing: ServiceCallback,
) {
    let registry = ProxyRegistry::default();
    registry
        .service_callbacks
        .lock()
        .unwrap()
        .push(existing.clone());
    register_on_proxy(&registry, &logger("later"));
    assert_eq!(callbacks(&registry), vec![existing]);
}

#[test]
fn registering_the_same_logger_twice_appends_it_once() {
    let registry = ProxyRegistry::default();
    register_on_proxy(&registry, &logger("first"));
    register_on_proxy(&registry, &logger("first"));
    assert_eq!(callbacks(&registry).len(), 1);
}

#[test]
fn the_first_registered_logger_keeps_the_proxy_slot() {
    let registry = ProxyRegistry::default();
    register_on_proxy(&registry, &logger("first"));
    register_on_proxy(&registry, &logger("second"));
    assert_eq!(proxy_logger(&registry), Some(logger("first")));
}

#[test]
fn a_filled_proxy_slot_is_kept_even_when_the_callback_list_is_empty() {
    let registry = ProxyRegistry::default();
    *registry.open_telemetry_logger.lock().unwrap() = Some(logger("owner"));
    register_on_proxy(&registry, &logger("second"));
    assert_eq!(proxy_logger(&registry), Some(logger("owner")));
    assert_eq!(
        callbacks(&registry),
        vec![ServiceCallback::OpenTelemetry(logger("second"))]
    );
}
