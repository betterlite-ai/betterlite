use std::error::Error as StdError;
use std::future::Future;
use std::pin::Pin;

use crate::autorouter_drain::TurnWriter;
use crate::baseline::store::BaselineAccounting;
use crate::guardrail_usage_rollups::GuardrailRollupSink;
use crate::model_usage::ModelUsageWriter;
use crate::request_usage::flush::{SpendLogsJob, SpendLogsJobError, queued_spend_transactions};
use crate::shadow_tables::rows::ShadowTables;
use crate::spend_log_storage::SpendLogsWriter;
use crate::spend_log_storage::rows::parking::recover_parked_spend_logs;
use crate::spend_log_storage::rows::writer::{SpendLogFailureSink, SpendUpdateClient};
use crate::tool_spend::ToolSpendWriter;

pub type ServiceFuture<'run, Value> = Pin<Box<dyn Future<Output = Value> + Send + 'run>>;

// What the spend log monitor and the shutdown drain ask of the job they drive: the memory census, the parked-row recovery and the two pass shapes.
pub trait SpendLogsService: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn recover_parked_logs(&self) -> ServiceFuture<'_, usize>;

    fn queued_transactions(&self) -> i64;

    fn update_spend_logs(&self) -> ServiceFuture<'_, Result<(), Self::Error>>;

    fn run_spend_logs(&self) -> ServiceFuture<'_, Result<(), Self::Error>>;
}

// update_spend_logs_job and _run_spend_logs_job as the lifecycle layer calls them: the guarded pass gates on the census itself, and the drain reaches the unguarded one.
impl<LogWriter, LogClient, LogFailures, Guardrails, Tools, Models, Turns, Accounting, Shadows>
    SpendLogsService
    for SpendLogsJob<
        '_,
        LogWriter,
        LogClient,
        LogFailures,
        Guardrails,
        Tools,
        Models,
        Turns,
        Accounting,
        Shadows,
    >
where
    LogWriter: SpendLogsWriter,
    LogClient: SpendUpdateClient,
    LogFailures: SpendLogFailureSink,
    Guardrails: GuardrailRollupSink,
    Tools: ToolSpendWriter,
    Models: ModelUsageWriter,
    Turns: TurnWriter,
    Accounting: BaselineAccounting,
    Shadows: ShadowTables,
{
    type Error = SpendLogsJobError<LogWriter::Error, LogClient::Error>;

    fn recover_parked_logs(&self) -> ServiceFuture<'_, usize> {
        Box::pin(recover_parked_spend_logs(self.queues.logs, self.logs.redis))
    }

    fn queued_transactions(&self) -> i64 {
        queued_spend_transactions(&self.queues)
    }

    fn update_spend_logs(&self) -> ServiceFuture<'_, Result<(), Self::Error>> {
        Box::pin(SpendLogsJob::update_spend_logs_job(self))
    }

    fn run_spend_logs(&self) -> ServiceFuture<'_, Result<(), Self::Error>> {
        Box::pin(SpendLogsJob::run_spend_logs_job(self))
    }
}
