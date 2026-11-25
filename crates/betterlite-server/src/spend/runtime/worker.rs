use std::sync::Arc;

use betterlite_db::daily_records::projection::{CallTypeEndpoints, DailyPricing};
use betterlite_db::spend_log_storage::postgres::PostgresSpendLogsWriter;
use betterlite_db::spend_queues::entity::SpendUpdateQueue;
use betterlite_db::spend_writer::accounting::{
    NoSpendAccountingAdmission, SpendAccounting, SpendAccountingEnvironment, batch_database_updates,
};
use betterlite_db::spend_writer::settings::SpendWriterSettings;
use betterlite_db::spend_writer::{DbSpendUpdateWriter, SpendWriterFailureSink, SpendWriterQueues};
use betterlite_error::proxy_model_not_found::route_endpoint;
use chrono::Utc;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::SpendRuntimeSettings;
use super::handles::RuntimeQueues;
use super::parts::{DailyQueues, SpendRuntimeParts, SpendUsageQueues};
use super::pricing::{PricingView, SpendPricingSource};

// The single owner of admitted accounting work: it writes every sibling in source order and drains what a shutdown leaves behind.
pub(super) struct AccountingWorker {
    pub(super) settings: SpendWriterSettings,
    pub(super) entity: Arc<SpendUpdateQueue>,
    pub(super) daily: DailyQueues,
    pub(super) usage: SpendUsageQueues,
    pub(super) pricing: Arc<dyn SpendPricingSource>,
    pub(super) failures: Arc<dyn SpendWriterFailureSink>,
    pub(super) shutdown: CancellationToken,
}

impl AccountingWorker {
    // The worker stops taking requests when the runtime closes its admission, then finishes what is already queued.
    pub(super) async fn run(self, mut receiver: Receiver<SpendAccounting>) {
        loop {
            let incoming = self.shutdown.run_until_cancelled(receiver.recv()).await;
            match incoming {
                None => break,
                Some(None) => return,
                Some(Some(work)) => self.account(work).await,
            }
        }
        while let Some(work) = receiver.recv().await {
            self.account(work).await;
        }
    }

    async fn account(&self, work: SpendAccounting) {
        let endpoints = RouteEndpoints;
        let view = PricingView(self.pricing.as_ref());
        let pricing = DailyPricing {
            context: &view,
            zone: self.pricing.zone(),
            fallback_moment: Utc::now(),
        };
        let environment = SpendAccountingEnvironment {
            pricing: &pricing,
            endpoints: &endpoints,
            deployments: None,
        };
        let admission = NoSpendAccountingAdmission;
        let writer: DbSpendUpdateWriter<'_, PostgresSpendLogsWriter> = DbSpendUpdateWriter {
            settings: self.settings.clone(),
            queues: SpendWriterQueues {
                entity: Some(self.entity.as_ref()),
                daily_user: Some(self.daily.user.as_ref()),
                daily_end_user: Some(self.daily.end_user.as_ref()),
                daily_agent: Some(self.daily.agent.as_ref()),
                daily_team: Some(self.daily.team.as_ref()),
                daily_org: Some(self.daily.org.as_ref()),
                daily_tag: Some(self.daily.tag.as_ref()),
                tool_usage: None,
                model_usage: self.usage.model_usage.as_deref(),
                discovery: None,
                turns: None,
            },
            attachment: None,
            failures: self.failures.as_ref(),
            admission: &admission,
        };
        batch_database_updates(&writer, &work, &environment).await;
    }
}

// The worker's handles and the bounded queue that admits its work, built once by SpendRuntime::new.
pub(super) fn spawn_accounting(
    settings: &SpendRuntimeSettings,
    parts: &SpendRuntimeParts,
    queues: &RuntimeQueues,
    shutdown: &CancellationToken,
) -> (Sender<SpendAccounting>, TaskTracker) {
    let worker = AccountingWorker {
        settings: settings.writer.clone(),
        entity: Arc::clone(&queues.entity),
        daily: queues.daily.clone(),
        usage: parts.usage.clone(),
        pricing: Arc::clone(&parts.pricing),
        failures: Arc::clone(&parts.failures),
        shutdown: shutdown.clone(),
    };
    let (sender, receiver) = mpsc::channel(settings.accounting_capacity);
    let tasks = TaskTracker::new();
    tasks.spawn(worker.run(receiver));
    (sender, tasks)
}

// ROUTE_ENDPOINT_MAPPING: the endpoint a call type belongs to, which the daily builders record with a row.
struct RouteEndpoints;

impl CallTypeEndpoints for RouteEndpoints {
    fn endpoint(&self, call_type: &str) -> Option<&str> {
        route_endpoint(call_type)
    }
}
