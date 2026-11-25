pub mod accounting;
pub mod request;
pub mod settings;

#[path = "spend_writer/tests/claim.rs"]
mod claim;
#[path = "spend_writer/tests/support.rs"]
mod support;

use std::error::Error as StdError;

use betterlite_models::tool_spend::ToolUsageTransaction;

use self::accounting::SpendAccountingAdmission;
use self::settings::SpendWriterSettings;
use crate::daily_records::callers::DailySpendQueue;
use crate::model_usage::ModelUsageTransaction;
use crate::request_usage::AutorouterQueues;
use crate::request_usage::queues::UsageQueue;
use crate::spend_attribution::enqueue::EntitySpendQueue;
use crate::spend_log_storage::batch_claim::{BatchClaimTable, SpendLogAttachment};
use crate::spend_queues::entity::SpendAmount;
use crate::tool_discovery::ToolDiscoveryQueue;

// The write side of update_database, named by the step that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpendWriterOperation {
    UpdateDatabase,
    KeyEnqueue,
    UserEnqueue,
}

// spend_log_error's fields: the request's attribution and the failure it caught.
pub struct SpendWriterFailure<'call> {
    pub operation: SpendWriterOperation,
    pub response_cost: Option<SpendAmount>,
    pub token: Option<&'call str>,
    pub user_id: Option<&'call str>,
    pub team_id: Option<&'call str>,
    pub org_id: Option<&'call str>,
    pub end_user_id: Option<&'call str>,
    pub error: &'call (dyn StdError + Send + Sync),
}

// B1708's spend_log_error behind a port: the writer names the request that failed and the sink decides how it is logged.
pub trait SpendWriterFailureSink: Send + Sync {
    fn report(&self, failure: SpendWriterFailure<'_>);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoSpendWriterFailureSink;

impl SpendWriterFailureSink for NoSpendWriterFailureSink {
    fn report(&self, _failure: SpendWriterFailure<'_>) {}
}

// The queues one request-time write reaches: the entity and daily spend queues, the ancillary usage buffers and the turn queues.
#[derive(Default)]
pub struct SpendWriterQueues<'work> {
    pub entity: Option<&'work dyn EntitySpendQueue>,
    pub daily_user: Option<&'work dyn DailySpendQueue>,
    pub daily_end_user: Option<&'work dyn DailySpendQueue>,
    pub daily_agent: Option<&'work dyn DailySpendQueue>,
    pub daily_team: Option<&'work dyn DailySpendQueue>,
    pub daily_org: Option<&'work dyn DailySpendQueue>,
    pub daily_tag: Option<&'work dyn DailySpendQueue>,
    pub tool_usage: Option<&'work UsageQueue<ToolUsageTransaction>>,
    pub model_usage: Option<&'work UsageQueue<ModelUsageTransaction>>,
    pub discovery: Option<&'work ToolDiscoveryQueue>,
    pub turns: Option<&'work AutorouterQueues<'work>>,
}

// DBSpendUpdateWriter's request-time half: prisma_client arrives as the claim attachment, and the queues it owns arrive as borrowed handles.
pub struct DbSpendUpdateWriter<'work, Table: BatchClaimTable + ?Sized> {
    pub settings: SpendWriterSettings,
    pub queues: SpendWriterQueues<'work>,
    pub attachment: Option<SpendLogAttachment<'work, Table>>,
    pub failures: &'work dyn SpendWriterFailureSink,
    pub admission: &'work dyn SpendAccountingAdmission,
}
