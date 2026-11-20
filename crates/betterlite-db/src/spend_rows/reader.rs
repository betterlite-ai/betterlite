use std::error::Error as StdError;
use std::future::Future;
use std::sync::Arc;

use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;

use crate::database_failures::deadline::{DbLookupError, LookupRequest, bounded_db_lookup};
use crate::database_failures::gate::{DbLookupStallTracker, LoopBoundSemaphore};

use super::key::{SpendCounterKey, WindowDurationValidity, decode_spend_counter_key};
use super::rows::{LookupAnswer, SpendCounterRows, SpendRow, SpendRowLookup};

pub const SPEND_COUNTER_LOOKUP: &str = "spend_counter";
pub const END_USER_SPEND_LOOKUP: &str = "end_user_spend";

// SpendCounterReseed as the two reads B1736 builds on: the primary entity row and the end-user budget floor
pub struct SpendCounterReader<Rows: SpendCounterRows> {
    pub rows: Option<Arc<Rows>>,
    pub durations: Arc<dyn WindowDurationValidity>,
    pub tasks: TaskTracker,
    pub gate: Arc<LoopBoundSemaphore>,
    pub stall_tracker: Arc<DbLookupStallTracker>,
    pub deadline_seconds: f64,
}

impl<Rows: SpendCounterRows> SpendCounterReader<Rows> {
    pub async fn from_db(&self, counter_key: &str) -> Option<Decimal> {
        let rows = self.rows.as_ref().map(Arc::clone)?;
        let key = counter_key.to_owned();
        let gate = Arc::clone(&self.gate);
        let durations = Arc::clone(&self.durations);
        let lookup = async move {
            let answer =
                gated_primary(gate.as_ref(), rows.as_ref(), durations.as_ref(), &key).await;
            Ok::<_, DbLookupError>(answer)
        };
        self.bounded_amount(lookup, SPEND_COUNTER_LOOKUP, counter_key)
            .await
    }

    // The floor read opens no db lookup gate, as end_user_from_db does not either; only the bounded lookup guards it
    pub async fn end_user_from_db(&self, counter_key: &str) -> Option<Decimal> {
        let rows = self.rows.as_ref().map(Arc::clone)?;
        let key = counter_key.to_owned();
        let durations = Arc::clone(&self.durations);
        let lookup = async move {
            let answer = end_user_answer(rows.as_ref(), durations.as_ref(), &key).await;
            Ok::<_, DbLookupError>(answer)
        };
        self.bounded_amount(lookup, END_USER_SPEND_LOOKUP, counter_key)
            .await
    }

    async fn bounded_amount<Failure, Lookup>(
        &self,
        lookup: Lookup,
        name: &str,
        counter_key: &str,
    ) -> Option<Decimal>
    where
        Failure: StdError + Send + Sync + 'static,
        Lookup: Future<Output = Result<LookupAnswer<SpendRow, Failure>, DbLookupError>>
            + Send
            + 'static,
    {
        let request = LookupRequest {
            name,
            deadline_seconds: None,
            default_deadline_seconds: self.deadline_seconds,
            tracker: self.stall_tracker.as_ref(),
        };
        match bounded_db_lookup(lookup, request, &self.tasks).await {
            Ok(LookupAnswer::Row(row)) => row.map(|stored| stored.spend.unwrap_or(Decimal::ZERO)),
            Ok(LookupAnswer::Failed(error)) => {
                tracing::error!(
                    %error,
                    counter_key,
                    operation = name,
                    "SpendCounterReseed: the counter row read failed"
                );
                None
            }
            Err(error) => {
                tracing::error!(
                    %error,
                    counter_key,
                    operation = name,
                    "SpendCounterReseed: the bounded lookup failed"
                );
                None
            }
        }
    }
}

async fn gated_primary<Rows: SpendCounterRows>(
    gate: &LoopBoundSemaphore,
    rows: &Rows,
    durations: &dyn WindowDurationValidity,
    counter_key: &str,
) -> LookupAnswer<SpendRow, Rows::Error> {
    let _permit = match gate.current() {
        Some(semaphore) => semaphore.acquire_owned().await.ok(),
        None => None,
    };
    let Ok(key) = decode_spend_counter_key(counter_key.as_bytes(), durations) else {
        return LookupAnswer::Row(None);
    };
    let Some(lookup) = primary_lookup(key) else {
        return LookupAnswer::Row(None);
    };
    match rows.spend_row(lookup).await {
        Ok(row) => LookupAnswer::Row(row),
        Err(error) => LookupAnswer::Failed(error),
    }
}

async fn end_user_answer<Rows: SpendCounterRows>(
    rows: &Rows,
    durations: &dyn WindowDurationValidity,
    counter_key: &str,
) -> LookupAnswer<SpendRow, Rows::Error> {
    let Ok(SpendCounterKey::EndUser(user_id)) =
        decode_spend_counter_key(counter_key.as_bytes(), durations)
    else {
        return LookupAnswer::Row(None);
    };
    match rows.spend_row(SpendRowLookup::EndUser(user_id)).await {
        Ok(row) => LookupAnswer::Row(row),
        Err(error) => LookupAnswer::Failed(error),
    }
}

// _counter_row's prefix order, minus the end-user prefix that only the floor reader accepts
fn primary_lookup(key: SpendCounterKey<'_>) -> Option<SpendRowLookup<'_>> {
    match key {
        SpendCounterKey::Key(token) => Some(SpendRowLookup::Key(token)),
        SpendCounterKey::TeamMember { user_id, team_id } => {
            Some(SpendRowLookup::TeamMember { user_id, team_id })
        }
        SpendCounterKey::Team(team_id) => Some(SpendRowLookup::Team(team_id)),
        SpendCounterKey::User(user_id) => Some(SpendRowLookup::User(user_id)),
        SpendCounterKey::Organization(organization_id) => {
            Some(SpendRowLookup::Organization(organization_id))
        }
        SpendCounterKey::Project(project_id) => Some(SpendRowLookup::Project(project_id)),
        SpendCounterKey::EndUser(_user_id)
        | SpendCounterKey::Window { .. }
        | SpendCounterKey::Unknown(_text) => None,
    }
}
