#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::spend_queue::SpendQueueSettings;
use betterlite_config::spend_storage::load_spend_storage_policy;
use betterlite_config::types::ProxyTrust;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::redis_spend::log_row::SpendLogRow;
use betterlite_db::request_usage::queues::UsageQueue;
use betterlite_db::spend_flush::monitor::SpendMonitorSettings;
use betterlite_db::spend_log_storage::postgres::PostgresSpendLogsWriter;
use betterlite_db::spend_queues::entity::SpendAmount;
use betterlite_db::spend_writer::SpendWriterOperation;
use betterlite_db::spend_writer::accounting::{
    SpendAccounting, SpendAccountingAdmission, SpendAccountingClosed,
};
use betterlite_db::spend_writer::settings::SpendWriterSettings;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use rust_decimal::Decimal;

use super::parts::SpendUsageQueues;
use super::pricing::PricingView;
use super::{SpendRuntime, SpendRuntimeError, SpendRuntimeSettings};
use crate::state::shared::Shared;
use crate::state::spend_runtime::Slot;

mod support;

use self::support::{FakeSpendLogs, RecordingFailures, StaticPayload};

const PAYLOAD: &str = r#"{"user":"u1","api_key":"k1","model":"gpt-4o","model_group":"gpt-4o","custom_llm_provider":"openai","prompt_tokens":3,"completion_tokens":4,"request_id":"req-b070","startTime":"2025-01-02T03:04:05Z","endTime":"2025-01-02T03:04:06Z","request_tags":"[]","metadata":"{}","status":"success","spend":0.25}"#;
const CONFIG: &str = "model_list:
  - {model_name: b070-spend, litellm_params: {model: openai/gpt-4o}}
";
const POLICY: &str = "general_settings:
  disable_spend_updates: true
  disable_spend_logs: null
litellm_settings:
  max_budget: 25.5
";

fn settings() -> SpendRuntimeSettings {
    SpendRuntimeSettings {
        writer: SpendWriterSettings::default(),
        queue: SpendQueueSettings {
            queue_capacity: 8,
            aggregation_threshold: 6,
            flush_count: 8,
            default_lock_ttl_seconds: 60,
        },
        monitor: SpendMonitorSettings::DEFAULT,
        log_queue_bytes: 4096,
        accounting_capacity: 8,
        use_redis_transaction_buffer: false,
        dequeue_count: 100,
    }
}

fn service() -> FakeSpendLogs {
    FakeSpendLogs {
        queued: AtomicI64::new(0),
        runs: AtomicUsize::new(0),
    }
}

#[tokio::test]
async fn drains_the_admitted_accounting_before_the_shutdown_returns() {
    let model_usage = Arc::new(UsageQueue::new(100).expect("the model usage queue"));
    let pricing = support::pricing();
    let failures = Arc::new(RecordingFailures {
        reported: Mutex::new(Vec::new()),
    });
    let mut parts = support::parts_with(Arc::clone(&pricing));
    parts.usage = SpendUsageQueues {
        tool_usage: None,
        model_usage: Some(Arc::clone(&model_usage)),
    };
    parts.failures = Arc::clone(&failures);
    let runtime = SpendRuntime::new(settings(), parts).expect("the runtime");
    let view = PricingView(pricing.as_ref());
    let request = support::request(&view);
    let payload = StaticPayload(PAYLOAD);
    for _round in 0..3 {
        assert!(runtime.writer().update_database(&payload, &request).await);
    }

    let logging = service();
    runtime
        .shutdown(&logging)
        .await
        .expect("the shutdown drains the accounting worker");

    assert_eq!(model_usage.len(), 3);
    assert_eq!(logging.runs.load(Ordering::SeqCst), 0);
    let charged = runtime
        .entity
        .flush_aggregated_transactions()
        .expect("the entity queue flushes");
    assert_eq!(
        charged.user_list_transactions.get("u1"),
        Some(&SpendAmount(Decimal::new(75, 2)))
    );
    assert_eq!(charged.key_list_transactions.len(), 1);
    assert!(failures.reported.lock().unwrap().is_empty());
}

#[tokio::test]
async fn acknowledges_an_admission_closed_by_the_shutdown_and_reports_it() {
    let pricing = support::pricing();
    let failures = Arc::new(RecordingFailures {
        reported: Mutex::new(Vec::new()),
    });
    let mut parts = support::parts_with(Arc::clone(&pricing));
    parts.failures = Arc::clone(&failures);
    let runtime = SpendRuntime::new(settings(), parts).expect("the runtime");
    let logging = service();
    runtime.shutdown(&logging).await.expect("the shutdown");

    let refused = SpendAccounting {
        response_cost: None,
        user_id: None,
        hashed_token: None,
        team_id: None,
        org_id: None,
        project_id: None,
        end_user_id: None,
        model_access_groups: Vec::new(),
        payload: SpendLogRow::new(),
    };
    assert!(matches!(
        runtime.admit(refused).await,
        Err(SpendAccountingClosed)
    ));

    let view = PricingView(pricing.as_ref());
    let request = support::request(&view);
    let payload = StaticPayload(PAYLOAD);
    assert!(runtime.writer().update_database(&payload, &request).await);
    assert_eq!(
        *failures.reported.lock().unwrap(),
        vec![SpendWriterOperation::UpdateDatabase]
    );
}

#[tokio::test]
async fn starts_one_monitor_and_the_shutdown_stops_it() {
    let runtime = SpendRuntime::new(settings(), support::parts_with(support::pricing()))
        .expect("the runtime");
    let monitor_settings = settings().monitor;
    let first = Arc::new(service());
    runtime
        .monitor
        .start(
            Arc::clone(&first),
            Arc::clone(&runtime.logs),
            monitor_settings,
        )
        .expect("the first monitor starts");
    let second = Arc::new(service());
    assert!(
        runtime
            .monitor
            .start(second, Arc::clone(&runtime.logs), monitor_settings)
            .is_err()
    );
    assert!(runtime.monitor.is_running());

    runtime
        .shutdown(first.as_ref())
        .await
        .expect("the shutdown stops the monitor");
    assert!(!runtime.monitor.is_running());
    assert_eq!(first.runs.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn keeps_a_claimed_log_row_until_the_shutdown_parks_it() {
    let pricing = support::pricing();
    let mut parts = support::parts_with(Arc::clone(&pricing));
    parts.spend_logs = Some(Arc::new(PostgresSpendLogsWriter {
        store: PostgresStore { pool: None },
    }));
    let runtime = SpendRuntime::new(settings(), parts).expect("the runtime");
    let view = PricingView(pricing.as_ref());
    let request = support::request(&view);
    let payload = StaticPayload(PAYLOAD);
    assert!(runtime.writer().update_database(&payload, &request).await);
    assert_eq!(runtime.logs.len(), 1);

    let logging = service();
    runtime
        .shutdown(&logging)
        .await
        .expect("the shutdown parks the row");
    assert_eq!(runtime.logs.len(), 1);
    assert_eq!(logging.runs.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn installs_the_runtime_once_and_maps_the_spend_policy() {
    let environment =
        BTreeMap::from([("LITELLM_ASYNCIO_QUEUE_MAXSIZE".to_owned(), "7".to_owned())]);
    let policy = load_spend_storage_policy(POLICY, &environment).expect("the spend policy");
    let settings =
        SpendRuntimeSettings::from_policy(&policy, &environment).expect("the runtime settings");
    assert!(settings.writer.disable_spend_updates);
    assert_eq!(settings.writer.disable_spend_logs, None);
    assert_eq!(settings.writer.max_budget, Decimal::new(255, 1));
    assert_eq!(settings.writer.proxy_budget_name, "litellm-proxy-budget");
    assert_eq!(settings.queue.queue_capacity, 7);
    assert_eq!(settings.queue.aggregation_threshold, 5);
    assert_eq!(settings.accounting_capacity, 7);

    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(CONFIG)
    .expect("the config");
    let policy =
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the proxy policy");
    let shared = Shared {
        bundled_catalog: BundledCatalog::default().get(),
        client: ProxiedClient::outbound(&Arc::new(policy)).expect("the client"),
    };
    let slot = Slot::new(&config, &shared);
    let pricing = support::pricing();
    slot.install(settings.clone(), support::parts_with(Arc::clone(&pricing)))
        .expect("the first install");
    let second = slot.install(settings, support::parts_with(pricing));
    assert!(matches!(second, Err(SpendRuntimeError::AlreadyInstalled)));
    assert!(slot.runtime.lock().unwrap().is_some());
}
