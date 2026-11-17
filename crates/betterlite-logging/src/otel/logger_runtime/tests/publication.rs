#![cfg(test)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};

use crate::callback_logger::CallbackLogger;
use crate::otel::logger_runtime::factory::build_otel_v2_logger;
use crate::otel::logger_runtime::publish::{fan_out_provider, publish_global_otel_v2_provider};
use crate::otel::logger_runtime::select::{
    excluded_db_systems, select_global_otel_v2_logger, v2_configs,
};
use crate::otel::logger_runtime::types::InjectedProviders;

use super::support::{
    FakeLogger, finished, fixture, fixture_excluding, lock, operator_exporter, placeholder, probe,
    register_callback, settings,
};

#[test]
fn registered_first() {
    let fixture = fixture();
    let sink = InMemorySpanExporter::default();
    let registered = register_callback(&fixture.registered, settings(vec![]), "registered", &sink);
    let other = build_otel_v2_logger(
        &fixture.runtime,
        settings(vec![]),
        Some("other"),
        InjectedProviders::default(),
    )
    .expect("built");
    let in_memory = vec![Arc::clone(&other.callback)];
    let selected = select_global_otel_v2_logger(&fixture.runtime, &in_memory).expect("selected");
    let registered_callback: Arc<dyn CallbackLogger> = Arc::clone(&registered);
    assert!(Arc::ptr_eq(&selected.callback, &registered_callback));
}

#[test]
fn first_v2_fallback() {
    let fixture = fixture();
    let legacy: Arc<dyn CallbackLogger> = Arc::new(FakeLogger {
        name: "legacy".to_owned(),
    });
    let first = build_otel_v2_logger(
        &fixture.runtime,
        settings(vec![]),
        Some("first"),
        InjectedProviders::default(),
    )
    .expect("built");
    let second = build_otel_v2_logger(
        &fixture.runtime,
        settings(vec![]),
        Some("second"),
        InjectedProviders::default(),
    )
    .expect("built");
    let in_memory = vec![
        legacy,
        Arc::clone(&first.callback),
        Arc::clone(&second.callback),
    ];
    let selected = select_global_otel_v2_logger(&fixture.runtime, &in_memory).expect("selected");
    assert!(Arc::ptr_eq(&selected.callback, &first.callback));
    assert_eq!(fixture.constructor.records().len(), 2);
}

#[test]
fn generic_once() {
    let fixture = fixture();
    let in_memory: Vec<Arc<dyn CallbackLogger>> = Vec::new();
    let first = select_global_otel_v2_logger(&fixture.runtime, &in_memory).expect("selected");
    let second = select_global_otel_v2_logger(&fixture.runtime, &in_memory).expect("selected");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(fixture.constructor.records().len(), 1);
    assert_eq!(fixture.constructor.records()[0].callback_name, None);
    assert_eq!(first.config, settings(vec![placeholder()]));
}

#[test]
fn legacy_slot_with_v2_catalog() {
    let fixture = fixture();
    let legacy: Arc<dyn CallbackLogger> = Arc::new(FakeLogger {
        name: "legacy".to_owned(),
    });
    let v2 = build_otel_v2_logger(
        &fixture.runtime,
        settings(vec![]),
        Some("v2"),
        InjectedProviders::default(),
    )
    .expect("built");
    let in_memory = vec![legacy, Arc::clone(&v2.callback)];
    let selected = select_global_otel_v2_logger(&fixture.runtime, &in_memory).expect("selected");
    assert!(Arc::ptr_eq(&selected.callback, &v2.callback));
    let configs = v2_configs(&fixture.runtime, &in_memory, &selected);
    assert_eq!(configs.len(), 1);
    assert_eq!(fixture.constructor.records().len(), 1);
}

#[test]
fn distinct_equal_configs() {
    let fixture = fixture();
    let config = settings(vec![operator_exporter()]);
    let first = build_otel_v2_logger(
        &fixture.runtime,
        config.clone(),
        Some("first"),
        InjectedProviders::default(),
    )
    .expect("built");
    let second = build_otel_v2_logger(
        &fixture.runtime,
        config.clone(),
        Some("second"),
        InjectedProviders::default(),
    )
    .expect("built");
    let in_memory = vec![Arc::clone(&first.callback), Arc::clone(&second.callback)];
    let selected = select_global_otel_v2_logger(&fixture.runtime, &in_memory).expect("selected");
    let configs = v2_configs(&fixture.runtime, &in_memory, &selected);
    assert_eq!(configs, vec![config.clone(), config]);
}

#[test]
fn explicit_empty_exclusions() {
    let fixture = fixture_excluding(Some(Vec::new()));
    let mut config = settings(vec![]);
    config.excluded_services = BTreeSet::from(["postgresql".to_owned()]);
    assert!(excluded_db_systems(&fixture.runtime, &config).is_empty());
}

#[test]
fn absent_exclusions_use_the_selected_config() {
    let fixture = fixture();
    let mut config = settings(vec![]);
    config.excluded_services = BTreeSet::from(["redis".to_owned()]);
    assert_eq!(
        excluded_db_systems(&fixture.runtime, &config),
        config.excluded_services
    );
}

#[test]
fn configured_exclusions_normalize() {
    let fixture = fixture_excluding(Some(vec![
        " Redis ".to_owned(),
        "postgres".to_owned(),
        "postgresql".to_owned(),
        "batch_write_to_db".to_owned(),
        "typo".to_owned(),
    ]));
    assert_eq!(
        excluded_db_systems(&fixture.runtime, &settings(vec![])),
        BTreeSet::from(["postgresql".to_owned(), "redis".to_owned()])
    );
}

#[test]
fn attach_before_publish() {
    let fixture = fixture();
    let unwired = fan_out_provider(&fixture.runtime);
    assert!(unwired.registry().fan_out.is_none());
    let fan_out = Arc::clone(&unwired.fan_out);
    let attached = Arc::new(Mutex::new(None));
    let received = Arc::new(Mutex::new(None));
    let setter: &dyn Fn(&SdkTracerProvider) = &|provider: &SdkTracerProvider| {
        *lock(&attached) = Some(lock(&fan_out).fan_out.is_some());
        *lock(&received) = Some(provider.clone());
    };
    let logger = build_otel_v2_logger(
        &fixture.runtime,
        settings(vec![]),
        Some("published"),
        InjectedProviders::default(),
    )
    .expect("built");
    let in_memory = vec![Arc::clone(&logger.callback)];
    let published =
        publish_global_otel_v2_provider(&fixture.runtime, &in_memory, setter).expect("published");
    assert_eq!(*lock(&attached), Some(true));
    let seen = lock(&received).clone().expect("the setter saw a provider");
    assert_eq!(fixture.constructor.sink_index(&seen), Some(0));
    assert_eq!(fixture.constructor.sink_index(&published.provider), Some(0));
    assert_eq!(finished(&fixture.sdk_sink), 0);
}

#[test]
fn remembered_provider_beats_global() {
    let fixture = fixture();
    let registered_sink = InMemorySpanExporter::default();
    register_callback(
        &fixture.registered,
        settings(vec![]),
        "registered",
        &registered_sink,
    );
    let published =
        publish_global_otel_v2_provider(&fixture.runtime, &[], &|_provider: &SdkTracerProvider| {})
            .expect("published");
    let provider = fan_out_provider(&fixture.runtime);
    assert!(provider.registry().fan_out.is_some());
    probe(&provider.provider);
    assert_eq!(finished(&registered_sink), 1);
    assert_eq!(finished(&fixture.sdk_sink), 0);
    probe(&published.provider);
    assert_eq!(finished(&registered_sink), 2);
}

#[test]
fn registered_fallback_attaches() {
    let fixture = fixture();
    let sink = InMemorySpanExporter::default();
    register_callback(&fixture.registered, settings(vec![]), "registered", &sink);
    let provider = fan_out_provider(&fixture.runtime);
    assert!(provider.registry().fan_out.is_some());
    probe(&provider.provider);
    assert_eq!(finished(&sink), 1);
    assert_eq!(finished(&fixture.sdk_sink), 0);
}

#[test]
fn sdk_fallback_builds_nothing() {
    let fixture = fixture();
    let provider = fan_out_provider(&fixture.runtime);
    assert!(provider.registry().fan_out.is_none());
    probe(&provider.provider);
    assert_eq!(finished(&fixture.sdk_sink), 1);
    assert_eq!(fixture.constructor.records().len(), 0);
}
