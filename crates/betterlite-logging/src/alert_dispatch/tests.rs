#![cfg(test)]

mod digest;
mod flush;
mod formatting;
mod routing;

use std::collections::BTreeMap;
use std::mem;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::alert_settings::types::{AlertType, AlertTypeConfig};
use betterlite_config::sections::alert_settings::webhooks::WebhookUrl;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset, TimeZone as _};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;

use super::budget::BudgetAlertSink;
use super::destination::{AlertDestination, SLACK_WEBHOOK_URL_ENV};
use super::{Alert, AlertDispatcher, AlertLevel};
use crate::alert_batching::alert_delivery::{AlertDelivery, FlushTask};
use crate::alert_digest::{Clock, DigestBuckets, DigestState};
use crate::alert_settings::records::DigestEntry;
use crate::batch::logger::{
    BatchQueue, BatchSettings, DEFAULT_FLUSH_INTERVAL_SECONDS, DEFAULT_MAX_QUEUE_SIZE,
};
use crate::budget_email::event::{EntityType, WebhookEvent, WebhookEventType};
use crate::http_analytics_loggers::delivery::{Delivery, PostRequest, PostResponse};

#[derive(Debug)]
struct FakeClock {
    now: Mutex<DateTime<FixedOffset>>,
}

impl FakeClock {
    fn new(now: DateTime<FixedOffset>) -> Self {
        Self {
            now: Mutex::new(now),
        }
    }

    fn set(&self, now: DateTime<FixedOffset>) {
        *self.now.lock().unwrap() = now;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<FixedOffset> {
        *self.now.lock().unwrap()
    }
}

fn time(hour: u32, minute: u32, second: u32) -> DateTime<FixedOffset> {
    FixedOffset::east_opt(0)
        .expect("UTC is a valid offset")
        .with_ymd_and_hms(2026, 1, 1, hour, minute, second)
        .single()
        .expect("the test time is unambiguous")
}

#[derive(Debug, thiserror::Error)]
#[error("the fake transport failed")]
struct FakeError;

struct FakeDelivery {
    requests: Mutex<Vec<PostRequest>>,
}

impl FakeDelivery {
    fn new() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    fn take_requests(&self) -> Vec<PostRequest> {
        mem::take(&mut *self.requests.lock().unwrap())
    }
}

impl Delivery for FakeDelivery {
    type Error = FakeError;

    async fn post(&self, request: PostRequest) -> Result<PostResponse, FakeError> {
        self.requests.lock().unwrap().push(request);
        Ok(PostResponse {
            status: 200,
            body: Vec::new(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SinkCall {
    Webhook(String),
    Email(String, AlertType),
}

#[derive(Debug, thiserror::Error)]
#[error("the fake sink failed")]
struct FakeSinkError;

struct FakeSink {
    calls: Mutex<Vec<SinkCall>>,
    fail_webhook: bool,
    fail_email: bool,
}

impl FakeSink {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fail_webhook: false,
            fail_email: false,
        }
    }

    fn calls(&self) -> Vec<SinkCall> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl BudgetAlertSink for FakeSink {
    type Error = FakeSinkError;

    async fn send_budget_webhook(&self, event: &WebhookEvent) -> Result<(), FakeSinkError> {
        self.calls
            .lock()
            .unwrap()
            .push(SinkCall::Webhook(event.event_message.clone()));
        if self.fail_webhook {
            return Err(FakeSinkError);
        }
        Ok(())
    }

    async fn send_budget_email(
        &self,
        event: &WebhookEvent,
        alert_type: AlertType,
    ) -> Result<(), FakeSinkError> {
        self.calls
            .lock()
            .unwrap()
            .push(SinkCall::Email(event.event_message.clone(), alert_type));
        if self.fail_email {
            return Err(FakeSinkError);
        }
        Ok(())
    }
}

struct Fixture {
    delivery: Arc<AlertDelivery<FakeDelivery>>,
    clock: Arc<FakeClock>,
    sink: FakeSink,
    tasks: TaskTracker,
    webhooks: BTreeMap<AlertType, WebhookUrl>,
    default_webhook_url: Option<WebhookUrl>,
    metadata: IndexMap<String, Json>,
    kwargs: IndexMap<String, Json>,
}

impl Fixture {
    fn dispatcher(
        &self,
        alerting: Option<Vec<AlertDestination>>,
        alert_types: Vec<AlertType>,
    ) -> AlertDispatcher<'_, FakeDelivery, FakeSink> {
        AlertDispatcher {
            alerting,
            alert_types,
            alert_to_webhook_url: self.webhooks.clone(),
            default_webhook_url: self.default_webhook_url.clone(),
            delivery: &self.delivery,
            sink: &self.sink,
            tasks: &self.tasks,
        }
    }

    fn alert<'borrow>(
        &'borrow self,
        message: &'borrow str,
        alert_type: AlertType,
    ) -> Alert<'borrow> {
        Alert {
            message,
            level: AlertLevel::High,
            alert_type,
            alerting_metadata: &self.metadata,
            user_info: None,
            request_model: None,
            api_base: None,
            kwargs: &self.kwargs,
        }
    }

    fn digest_entries(&self) -> IndexMap<String, DigestEntry> {
        self.delivery.digests.state.lock().unwrap().entries.clone()
    }
}

fn build_delivery(
    clock: &Arc<FakeClock>,
    environment: &[(&str, &str)],
    config: BTreeMap<AlertType, AlertTypeConfig>,
    batch_size: usize,
) -> Arc<AlertDelivery<FakeDelivery>> {
    let clock_trait: Arc<dyn Clock> = Arc::clone(clock);
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let resolved: Arc<dyn EnvironmentLookup + Send + Sync> = Arc::new(lookup);
    let settings = BatchSettings {
        batch_size,
        flush_interval: Duration::from_secs(DEFAULT_FLUSH_INTERVAL_SECONDS),
        max_queue_size: DEFAULT_MAX_QUEUE_SIZE,
    };
    Arc::new(AlertDelivery {
        queue: BatchQueue::new(settings),
        digests: DigestBuckets {
            state: Mutex::new(DigestState {
                config,
                entries: IndexMap::new(),
            }),
            clock: clock_trait,
        },
        delivery: FakeDelivery::new(),
        log_to_console: false,
        environment: resolved,
        flush_task: FlushTask::default(),
    })
}

fn build(
    environment: &[(&str, &str)],
    config: BTreeMap<AlertType, AlertTypeConfig>,
    batch_size: usize,
) -> Fixture {
    let clock = Arc::new(FakeClock::new(time(12, 0, 0)));
    Fixture {
        delivery: build_delivery(&clock, environment, config, batch_size),
        clock,
        sink: FakeSink::new(),
        tasks: TaskTracker::new(),
        webhooks: BTreeMap::new(),
        default_webhook_url: None,
        metadata: IndexMap::new(),
        kwargs: IndexMap::new(),
    }
}

fn fixture(environment: &[(&str, &str)], batch_size: usize) -> Fixture {
    build(environment, BTreeMap::new(), batch_size)
}

fn digest_fixture(
    environment: &[(&str, &str)],
    batch_size: usize,
    digest_types: &[AlertType],
) -> Fixture {
    let config: BTreeMap<AlertType, AlertTypeConfig> = digest_types
        .iter()
        .map(|alert_type| {
            (
                *alert_type,
                AlertTypeConfig {
                    digest: true,
                    digest_interval: 86_400,
                },
            )
        })
        .collect();
    build(environment, config, batch_size)
}

fn budget_event() -> WebhookEvent {
    WebhookEvent {
        event: WebhookEventType::BudgetCrossed,
        event_message: "budget_crossed".to_owned(),
        event_group: EntityType::User,
        spend: Decimal::from(12),
        max_budget: Some(Decimal::from(100)),
        soft_budget: None,
        token: None,
        customer_id: None,
        user_id: Some("user-1".to_owned()),
        team_id: None,
        team_alias: None,
        organization_id: None,
        user_email: Some("user@example.com".to_owned()),
        key_alias: None,
        projected_exceeded_date: None,
        projected_spend: None,
        alert_emails: None,
        max_budget_alert_emails: None,
    }
}

fn slack_fixture() -> Fixture {
    fixture(&[(SLACK_WEBHOOK_URL_ENV, "https://hooks/slack")], 512)
}
