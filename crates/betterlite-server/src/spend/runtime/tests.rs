#![cfg(test)]

use std::collections::BTreeMap;
use std::ptr::eq;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::spend_queue::SpendQueueSettings;
use betterlite_config::spend_storage::load_spend_storage_policy;
use betterlite_config::types::ProxyTrust;
use betterlite_db::daily_records::projection::DailyPricing;
use betterlite_db::daily_spend_sql::values::SpendColumnValue;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::redis_spend::codec::decode_log_record;
use betterlite_db::redis_spend::events::NoRedisBufferEventSink;
use betterlite_db::redis_spend::log_row::SpendLogRow;
use betterlite_db::request_usage::queues::UsageQueue;
use betterlite_db::spend_flush::monitor::SpendMonitorSettings;
use betterlite_db::spend_flush::shutdown::MonitorStartError;
use betterlite_db::spend_log_storage::postgres::PostgresSpendLogsWriter;
use betterlite_db::spend_queues::entity::SpendAmount;
use betterlite_db::spend_writer::SpendWriterOperation;
use betterlite_db::spend_writer::accounting::{
    SpendAccounting, SpendAccountingAdmission as _, SpendAccountingClosed,
    SpendAccountingEnvironment, batch_database_updates,
};
use betterlite_db::spend_writer::settings::SpendWriterSettings;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::savings_pricing::context::{ModelIdentity, SavingsPricingContext as _};
use chrono::Utc;
use rust_decimal::Decimal;

use super::parts::SpendUsageQueues;
use super::pricing::{PricingView, SpendPricingSource};
use super::{SpendRuntime, SpendRuntimeError, SpendRuntimeSettings};
use crate::cost_estimate_route::testing::service_from;
use crate::state::shared::Shared;
use crate::state::spend_runtime::Slot;

mod support;

use self::support::{FakeSpendLogs, StaticPayload};

const PAYLOAD: &str = r#"{"user":"u1","api_key":"k1","model":"gpt-4o","model_group":"gpt-4o","custom_llm_provider":"openai","prompt_tokens":3,"completion_tokens":4,"request_id":"req-b070","startTime":"2025-01-02T03:04:05Z","endTime":"2025-01-02T03:04:06Z","request_tags":"[]","metadata":"{}","status":"success","spend":0.25}"#;
const ACCOUNTING_ROW: &str = r#"{"user":"u1","api_key":"k1","model":"gpt-4o","model_group":"gpt-4o","custom_llm_provider":"openai","call_type":"acompletion","prompt_tokens":3,"completion_tokens":4,"request_id":"req-b070-accounting","startTime":"2025-01-02T03:04:05Z","endTime":"2025-01-02T03:04:06Z","request_tags":"[\"tag-a\",\"tag-b\"]","metadata":"{}","status":"success","spend":0.25,"team_id":"team-1","organization_id":"org-1","end_user":"eu-1","agent_id":"agent-1"}"#;
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

fn accounting_row() -> SpendLogRow {
    decode_log_record(ACCOUNTING_ROW.as_bytes())
        .expect("the accounting row decodes")
        .expect("the accounting row is a JSON object")
}

fn accounting_work() -> SpendAccounting {
    SpendAccounting {
        response_cost: Some(SpendAmount(Decimal::new(25, 2))),
        user_id: Some("u1".to_owned()),
        hashed_token: Some("hash-1".to_owned()),
        team_id: Some("team-1".to_owned()),
        org_id: Some("org-1".to_owned()),
        project_id: Some("proj-1".to_owned()),
        end_user_id: Some("eu-1".to_owned()),
        model_access_groups: vec!["g1".to_owned(), "g2".to_owned()],
        payload: accounting_row(),
    }
}

#[tokio::test]
async fn drains_the_admitted_accounting_before_the_shutdown_returns() {
    let model_usage = Arc::new(UsageQueue::new(100).expect("the model usage queue"));
    let pricing = support::pricing();
    let (failures, sink) = support::recording_failures();
    let mut parts = support::parts_with(Arc::clone(&pricing));
    parts.usage = SpendUsageQueues {
        tool_usage: None,
        model_usage: Some(Arc::clone(&model_usage)),
    };
    parts.failures = sink;
    let runtime = SpendRuntime::new(settings(), parts).expect("the runtime");
    let request = support::request();
    let payload = StaticPayload(PAYLOAD);
    assert!(runtime.writer().update_database(&payload, &request).await);
    assert!(runtime.writer().update_database(&payload, &request).await);
    assert!(runtime.writer().update_database(&payload, &request).await);

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
    let (failures, sink) = support::recording_failures();
    let mut parts = support::parts_with(Arc::clone(&pricing));
    parts.failures = sink;
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

    let request = support::request();
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
    assert!(matches!(
        runtime
            .monitor
            .start(second, Arc::clone(&runtime.logs), monitor_settings),
        Err(MonitorStartError::AlreadyRunning)
    ));
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
    let request = support::request();
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
    let proxy_policy =
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the proxy policy");
    let shared = Shared {
        bundled_catalog: BundledCatalog::default().get(),
        client: ProxiedClient::outbound(&Arc::new(proxy_policy)).expect("the client"),
    };
    let slot = Slot::new(&config, &shared);
    let pricing = support::pricing();
    slot.install(settings.clone(), support::parts_with(Arc::clone(&pricing)))
        .expect("the first install");
    let second = slot.install(settings, support::parts_with(pricing));
    assert!(matches!(second, Err(SpendRuntimeError::AlreadyInstalled)));
    assert!(slot.runtime.lock().unwrap().is_some());
}

#[tokio::test]
async fn charges_every_accounting_queue_the_runtime_writer_was_built_with() {
    let model_usage = Arc::new(UsageQueue::new(100).expect("the model usage queue"));
    let source = support::pricing();
    let mut parts = support::parts_with(Arc::clone(&source));
    parts.usage = SpendUsageQueues {
        tool_usage: None,
        model_usage: Some(Arc::clone(&model_usage)),
    };
    let runtime = SpendRuntime::new(settings(), parts).expect("the runtime");
    let view = PricingView(source.as_ref());
    let pricing = DailyPricing {
        context: &view,
        zone: source.zone(),
        fallback_moment: Utc::now(),
    };
    let endpoints = support::Endpoints;
    let environment = SpendAccountingEnvironment {
        pricing: &pricing,
        endpoints: &endpoints,
        deployments: None,
    };
    let writer = runtime.writer();
    batch_database_updates(&writer, &accounting_work(), &environment).await;
    let charged = runtime
        .entity
        .flush_aggregated_transactions()
        .expect("the entity queue flushes");
    assert_eq!(
        charged.user_list_transactions.get("u1"),
        Some(&SpendAmount(Decimal::new(25, 2)))
    );
    assert_eq!(charged.key_list_transactions.len(), 1);
    let flushed = [
        runtime
            .daily
            .user
            .flush_aggregated_daily_rows()
            .await
            .expect("the daily user queue flushes")
            .len(),
        runtime
            .daily
            .end_user
            .flush_aggregated_daily_rows()
            .await
            .expect("the daily end user queue flushes")
            .len(),
        runtime
            .daily
            .agent
            .flush_aggregated_daily_rows()
            .await
            .expect("the daily agent queue flushes")
            .len(),
        runtime
            .daily
            .team
            .flush_aggregated_daily_rows()
            .await
            .expect("the daily team queue flushes")
            .len(),
        runtime
            .daily
            .org
            .flush_aggregated_daily_rows()
            .await
            .expect("the daily organization queue flushes")
            .len(),
        runtime
            .daily
            .tag
            .flush_aggregated_daily_rows()
            .await
            .expect("the daily tag queue flushes")
            .len(),
    ];
    assert_eq!(flushed, [1, 1, 1, 1, 1, 2]);
    assert_eq!(model_usage.len(), 1);
    runtime.shutdown(&service()).await.expect("the shutdown");
}

#[tokio::test]
async fn writes_the_request_tool_usage_and_discovery_from_the_runtime_queues() {
    let tool_usage = Arc::new(UsageQueue::new(10).expect("the tool usage queue"));
    let source = support::pricing();
    let mut parts = support::parts_with(Arc::clone(&source));
    parts.usage = SpendUsageQueues {
        tool_usage: Some(Arc::clone(&tool_usage)),
        model_usage: None,
    };
    let runtime = SpendRuntime::new(settings(), parts).expect("the runtime");
    let kwargs = JsonObject::new();
    let names = vec!["lookup".to_owned()];
    let mut request = support::request();
    request.kwargs = Some(&kwargs);
    request.response_names = &names;
    assert!(
        runtime
            .writer()
            .update_database(&StaticPayload(PAYLOAD), &request)
            .await
    );
    assert_eq!(tool_usage.len(), 1);
    let tools: Vec<Option<String>> = runtime
        .discovery
        .flush()
        .into_iter()
        .map(|item| item.tool_name)
        .collect();
    assert_eq!(tools, vec![Some("lookup".to_owned())]);
    runtime.shutdown(&service()).await.expect("the shutdown");
}

#[tokio::test]
async fn charges_every_runtime_daily_queue_and_records_the_endpoint() {
    let source = support::pricing();
    let runtime = SpendRuntime::new(settings(), support::parts_with(Arc::clone(&source)))
        .expect("the runtime");
    let mut request = support::request();
    request.team_id = Some("team-1");
    request.org_id = Some("org-1");
    assert!(
        runtime
            .writer()
            .update_database(&StaticPayload(ACCOUNTING_ROW), &request)
            .await
    );
    runtime.shutdown(&service()).await.expect("the shutdown");

    let user = runtime
        .daily
        .user
        .flush_aggregated_daily_rows()
        .await
        .expect("the daily user queue flushes");
    assert_eq!(user.len(), 1);
    let row = user.values().next().expect("the daily user row");
    assert_eq!(
        row.get("endpoint"),
        Some(&SpendColumnValue::Text("/chat/completions".to_owned()))
    );
    assert_eq!(
        runtime
            .daily
            .end_user
            .flush_aggregated_daily_rows()
            .await
            .expect("the daily end user queue flushes")
            .len(),
        1
    );
    assert_eq!(
        runtime
            .daily
            .agent
            .flush_aggregated_daily_rows()
            .await
            .expect("the daily agent queue flushes")
            .len(),
        1
    );
    assert_eq!(
        runtime
            .daily
            .team
            .flush_aggregated_daily_rows()
            .await
            .expect("the daily team queue flushes")
            .len(),
        1
    );
    assert_eq!(
        runtime
            .daily
            .org
            .flush_aggregated_daily_rows()
            .await
            .expect("the daily organization queue flushes")
            .len(),
        1
    );
    assert_eq!(
        runtime
            .daily
            .tag
            .flush_aggregated_daily_rows()
            .await
            .expect("the daily tag queue flushes")
            .len(),
        2
    );
}

#[tokio::test]
async fn builds_both_redis_list_handles_from_a_cache() {
    let cache = support::redis_cache().await;
    let (buffer, publisher) =
        super::handles::build_redis(&settings(), Some(&cache), Arc::new(NoRedisBufferEventSink));
    assert!(buffer.lists.is_some());
    assert!(publisher.lists.is_some());
    let (no_buffer, no_publisher) =
        super::handles::build_redis(&settings(), None, Arc::new(NoRedisBufferEventSink));
    assert!(no_buffer.lists.is_none());
    assert!(no_publisher.lists.is_none());
}

#[test]
fn resolves_the_model_through_the_real_cost_service() {
    let bytes = (BundledCatalog::default().asset)().expect("the bundled catalog is readable");
    let service = service_from(&bytes).expect("the cost service builds");
    assert_eq!(
        service.resolve_model("gpt-4o", Some("openai")),
        Some(ModelIdentity {
            model: "gpt-4o".to_owned(),
            provider: "openai".to_owned(),
        })
    );
    assert!(eq(SpendPricingSource::zone(&service), &service.database));
}

#[test]
fn delegates_model_identity_through_the_pricing_view() {
    let source = support::pricing();
    let view = PricingView(source.as_ref());
    assert_eq!(
        view.resolve_model("gpt-4o", Some("openai")),
        Some(ModelIdentity {
            model: "gpt-4o".to_owned(),
            provider: "openai".to_owned(),
        })
    );
}
