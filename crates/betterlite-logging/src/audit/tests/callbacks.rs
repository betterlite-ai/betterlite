#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::loader::{ConfigLoader, EnvironmentLookup};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::settings::JsonValue;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::super::callbacks::{
    AuditLoggerBuildError, CatalogAuditLoggerBuilder, DedicatedAuditLoggerBuilder,
    NamedAuditCallbacks,
};
use super::support::{
    FailingBuilder, FakeBuilder, FakeResolver, RecordingLogger, capture, payload,
};

fn named(
    resolver: Arc<FakeResolver>,
    dedicated: Arc<dyn DedicatedAuditLoggerBuilder>,
) -> NamedAuditCallbacks {
    NamedAuditCallbacks {
        resolver,
        dedicated,
        cache: Mutex::new(BTreeMap::new()),
    }
}

fn params(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

fn cached(callbacks: &NamedAuditCallbacks) -> Vec<String> {
    callbacks.cache.lock().unwrap().keys().cloned().collect()
}

#[test]
fn a_resolved_name_is_cached_after_the_first_lookup() {
    let logger = Arc::new(RecordingLogger::new("otel", false));
    let resolver = Arc::new(FakeResolver::new(vec![("otel", logger)], false));
    let callbacks = named(Arc::clone(&resolver), Arc::new(FailingBuilder));
    let first = callbacks.resolve("otel", None).expect("the name resolves");
    let second = callbacks.resolve("otel", None).expect("the cache answers");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(*resolver.calls.lock().unwrap(), vec!["otel".to_owned()]);
    assert_eq!(cached(&callbacks), vec!["otel".to_owned()]);
}

#[test]
fn an_unknown_name_warns_and_is_not_cached() {
    let (captured, guard) = capture();
    let resolver = Arc::new(FakeResolver::new(Vec::new(), false));
    let callbacks = named(Arc::clone(&resolver), Arc::new(FailingBuilder));
    assert!(callbacks.resolve("missing", None).is_none());
    assert!(callbacks.resolve("missing", None).is_none());
    drop(guard);
    assert_eq!(resolver.calls.lock().unwrap().len(), 2);
    assert!(cached(&callbacks).is_empty());
    assert!(
        captured
            .text()
            .contains("Could not resolve audit log callback: missing")
    );
}

#[test]
fn a_refused_resolver_warns_and_is_not_cached() {
    let (captured, guard) = capture();
    let resolver = Arc::new(FakeResolver::new(Vec::new(), true));
    let callbacks = named(Arc::clone(&resolver), Arc::new(FailingBuilder));
    assert!(callbacks.resolve("otel", None).is_none());
    assert!(callbacks.resolve("otel", None).is_none());
    drop(guard);
    assert_eq!(resolver.calls.lock().unwrap().len(), 2);
    assert!(cached(&callbacks).is_empty());
    assert!(
        captured
            .text()
            .contains("Could not resolve audit log callback: otel")
    );
}

#[test]
fn s3_v2_with_audit_params_builds_a_dedicated_logger() {
    let shared = Arc::new(RecordingLogger::new("s3_v2", false));
    let dedicated = Arc::new(RecordingLogger::new("dedicated", false));
    let resolver = Arc::new(FakeResolver::new(vec![("s3_v2", shared)], false));
    let builder = Arc::new(FakeBuilder::new(Arc::clone(&dedicated)));
    let callbacks = named(Arc::clone(&resolver), Arc::<FakeBuilder>::clone(&builder));
    let audit_params = params(r#"{"s3_bucket_name": "audit-bucket"}"#);
    let first = callbacks
        .resolve("s3_v2", Some(&audit_params))
        .expect("the dedicated logger builds");
    let expected: Arc<dyn crate::callback_logger::CallbackLogger> = dedicated;
    assert!(Arc::ptr_eq(&first, &expected));
    let second = callbacks
        .resolve("s3_v2", Some(&audit_params))
        .expect("cached");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(*builder.calls.lock().unwrap(), vec![audit_params]);
    assert!(resolver.calls.lock().unwrap().is_empty());
}

#[test]
fn s3_v2_without_audit_params_uses_the_shared_resolver() {
    let shared = Arc::new(RecordingLogger::new("s3_v2", false));
    let expected: Arc<dyn crate::callback_logger::CallbackLogger> = shared;
    let resolver = Arc::new(FakeResolver::new(
        vec![("s3_v2", Arc::clone(&expected))],
        false,
    ));
    let builder = Arc::new(FakeBuilder::new(Arc::new(RecordingLogger::new("x", false))));
    let callbacks = named(Arc::clone(&resolver), Arc::<FakeBuilder>::clone(&builder));
    let resolved = callbacks
        .resolve("s3_v2", None)
        .expect("the resolver answers");
    assert!(Arc::ptr_eq(&resolved, &expected));
    assert!(builder.calls.lock().unwrap().is_empty());
    assert_eq!(*resolver.calls.lock().unwrap(), vec!["s3_v2".to_owned()]);
}

#[test]
fn an_empty_s3_override_still_selects_the_dedicated_logger() {
    let dedicated = Arc::new(RecordingLogger::new("dedicated", false));
    let expected: Arc<dyn crate::callback_logger::CallbackLogger> =
        Arc::<RecordingLogger>::clone(&dedicated);
    let resolver = Arc::new(FakeResolver::new(
        vec![("s3_v2", Arc::new(RecordingLogger::new("shared", false)))],
        false,
    ));
    let builder = Arc::new(FakeBuilder::new(dedicated));
    let callbacks = named(Arc::clone(&resolver), Arc::<FakeBuilder>::clone(&builder));
    let empty = params("{}");
    let resolved = callbacks.resolve("s3_v2", Some(&empty)).expect("dedicated");
    assert!(Arc::ptr_eq(&resolved, &expected));
    assert_eq!(*builder.calls.lock().unwrap(), vec![empty]);
    assert!(resolver.calls.lock().unwrap().is_empty());
}

#[test]
fn a_failed_dedicated_build_is_logged_and_not_cached() {
    let (captured, guard) = capture();
    let resolver = Arc::new(FakeResolver::new(Vec::new(), false));
    let callbacks = named(Arc::clone(&resolver), Arc::new(FailingBuilder));
    let audit_params = params(r#"{"s3_bucket_name": "audit-bucket"}"#);
    assert!(callbacks.resolve("s3_v2", Some(&audit_params)).is_none());
    assert!(callbacks.resolve("s3_v2", Some(&audit_params)).is_none());
    drop(guard);
    assert!(cached(&callbacks).is_empty());
    assert!(resolver.calls.lock().unwrap().is_empty());
    assert!(
        captured
            .text()
            .contains("Failed dispatching audit log to callback")
    );
}

#[test]
fn reset_clears_the_cache_and_the_next_lookup_resolves_again() {
    let logger = Arc::new(RecordingLogger::new("otel", false));
    let resolver = Arc::new(FakeResolver::new(vec![("otel", logger)], false));
    let callbacks = named(Arc::clone(&resolver), Arc::new(FailingBuilder));
    assert!(callbacks.resolve("otel", None).is_some());
    callbacks.reset_audit_log_callback_cache();
    assert!(cached(&callbacks).is_empty());
    assert!(callbacks.resolve("otel", None).is_some());
    assert_eq!(resolver.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn the_real_builder_builds_from_the_audit_parameters() {
    let inputs = inputs("model_list: []\n");
    let source = build_source(
        r#"{"s3_bucket_name":"audit-bucket","s3_region_name":"us-east-1","s3_endpoint_url":"http://127.0.0.1:9000","s3_path":"audit-logs"}"#,
    );
    let builder = CatalogAuditLoggerBuilder { inputs };
    let logger = builder.build(&source).expect("the dedicated logger builds");
    assert_eq!(logger.callback_name(), "s3_v2");
    logger
        .async_log_audit_event(&payload())
        .await
        .expect("the s3 logger accepts an audit record");
    builder.inputs.shutdown.cancel();
    builder.inputs.tracker.close();
    timeout(Duration::from_secs(1), builder.inputs.tracker.wait())
        .await
        .expect("the flush loop stops");
}

#[tokio::test]
async fn the_real_builder_refuses_an_audit_source_without_a_bucket() {
    let builder = CatalogAuditLoggerBuilder {
        inputs: inputs("model_list: []\n"),
    };
    let error = builder
        .build(&build_source("{}"))
        .err()
        .expect("a bucket is required");
    assert!(
        matches!(error, AuditLoggerBuildError::Build { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn the_real_builder_refuses_non_object_parameters() {
    let builder = CatalogAuditLoggerBuilder {
        inputs: inputs("model_list: []\n"),
    };
    let error = builder
        .build(&build_source("\"not an object\""))
        .err()
        .expect("only an object describes S3");
    assert!(
        matches!(error, AuditLoggerBuildError::Parameters { .. }),
        "{error:?}"
    );
}

fn build_source(text: &str) -> JsonValue {
    match serde_json::from_str(text) {
        Ok(value) => value,
        Err(error) => panic!("the audit source is valid JSON: {error}"),
    }
}

fn inputs(source_text: &str) -> crate::registry::catalog::CatalogInputs {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source_text)
    .expect("the config loads");
    let callback_settings = config.sections.callback_settings.callback_settings.clone();
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    crate::registry::catalog::CatalogInputs {
        config: Arc::new(config),
        http: ProxiedClient::outbound(&policy).expect("the HTTP client builds"),
        env: credentials(),
        callback_settings,
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

fn credentials() -> Arc<dyn EnvironmentLookup + Send + Sync> {
    Arc::new(BTreeMap::from([
        ("AWS_ACCESS_KEY_ID".to_owned(), "AKIDTEST".to_owned()),
        ("AWS_SECRET_ACCESS_KEY".to_owned(), "secrettest".to_owned()),
    ]))
}
