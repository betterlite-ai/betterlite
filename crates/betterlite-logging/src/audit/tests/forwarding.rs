#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_config::loader::{ConfigLoader, EnvironmentLookup};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use secrecy::SecretString;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use url::Url;

use super::support::payload;
use crate::azure_sentinel_logging::logger::{AzureSentinelLogger, SentinelDeps};
use crate::azure_sentinel_logging::settings::{SentinelInit, SentinelSettings};
use crate::callback_logger::CallbackLogger;
use crate::datadog_logging::clock::SystemClock;
use crate::datadog_logging::transport::{
    PostRequest, Transport, TransportFailure, TransportResponse,
};
use crate::registry::catalog::CatalogInputs;
use crate::s3_audit::dedicated::build_audit_logger;
use crate::s3_v2::source::Source;

const AUDIT_SOURCE: &str = r#"{"s3_bucket_name":"audit-bucket","s3_region_name":"us-east-1","s3_endpoint_url":"http://127.0.0.1:9000","s3_path":"audit-logs"}"#;

struct NoTransport;

#[async_trait::async_trait]
impl Transport for NoTransport {
    async fn post(&self, _request: PostRequest) -> Result<TransportResponse, TransportFailure> {
        Err(TransportFailure::Url(
            Url::parse("not a url").expect_err("the text is not a URL"),
        ))
    }
}

#[tokio::test]
async fn the_s3_v2_logger_forwards_audit_events_to_the_s3_queue() {
    let inputs = inputs("model_list: []\n");
    let source: Source<()> = serde_json::from_str(AUDIT_SOURCE).expect("the audit source loads");
    let logger = build_audit_logger(&inputs, &source).expect("the audit logger builds");
    logger
        .async_log_audit_log_event(&payload())
        .await
        .expect("the s3 logger accepts an audit record");
    let queued = logger.queue.snapshot();
    assert_eq!(queued.entries.len(), 1);
    assert!(
        queued.entries[0]
            .s3_object_key
            .starts_with("audit-logs/audit_logs/"),
        "{}",
        queued.entries[0].s3_object_key
    );
    inputs.shutdown.cancel();
    inputs.tracker.close();
    timeout(Duration::from_secs(1), inputs.tracker.wait())
        .await
        .expect("the flush loop stops");
}

#[tokio::test]
async fn the_azure_sentinel_logger_queues_audit_events_on_its_audit_stream() {
    let settings = sentinel_settings();
    let logger = AzureSentinelLogger::new(
        settings,
        SentinelDeps {
            transport: Arc::new(NoTransport),
            clock: Arc::new(SystemClock),
            stopped: CancellationToken::new(),
            tasks: None,
        },
    );
    CallbackLogger::async_log_audit_log_event(&*logger, &payload())
        .await
        .expect("the sentinel logger accepts an audit record");
    let audit = logger
        .audit
        .queue
        .queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].id, "audit-1");
    drop(audit);
    let model = logger
        .model
        .queue
        .queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert!(model.is_empty());
}

fn sentinel_settings() -> SentinelSettings {
    let environment = BTreeMap::from([
        (
            "AZURE_SENTINEL_DCR_IMMUTABLE_ID".to_owned(),
            "dcr-1".to_owned(),
        ),
        (
            "AZURE_SENTINEL_ENDPOINT".to_owned(),
            "https://dce.example.test".to_owned(),
        ),
        ("AZURE_SENTINEL_TENANT_ID".to_owned(), "tenant-1".to_owned()),
        ("AZURE_SENTINEL_CLIENT_ID".to_owned(), "client-1".to_owned()),
    ]);
    let init = SentinelInit {
        dcr_immutable_id: None,
        stream_name: None,
        endpoint: None,
        tenant_id: None,
        client_id: None,
        client_secret: Some(SecretString::from("secret-1".to_owned())),
        audit_stream_name: None,
        authority_host: None,
    };
    SentinelSettings::resolve(&init, &environment).expect("the sentinel settings resolve")
}

fn inputs(source_text: &str) -> CatalogInputs {
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
    CatalogInputs {
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
