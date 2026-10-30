use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::error::Error;
use std::future::Future;

pub mod batch;
pub mod queue;
mod tests;

use batch::{BudgetCascadeBatch, CommitError, SpendResetBatch};
use queue::{
    BudgetWindowWrites, KeyLinkedSpendResetWrites, KeySpendResetWrites, LinkedSpendResetWrites,
    Queue, TeamSpendResetWrites, UserSpendResetWrites,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct SpendDecrement {
    pub decrement: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct SpendResetData {
    pub spend: SpendDecrement,
    pub budget_reset_at: Option<DateTime<Utc>>,
}

#[must_use]
pub const fn spend_reset_data(
    budget_reset_at: Option<DateTime<Utc>>,
    spend_decrement: Decimal,
) -> SpendResetData {
    let spend = SpendDecrement {
        decrement: spend_decrement,
    };
    SpendResetData {
        spend,
        budget_reset_at,
    }
}

#[derive(Debug)]
pub struct SpendResetUnitOfWork {
    pub keys: KeySpendResetWrites,
    pub users: UserSpendResetWrites,
    pub teams: TeamSpendResetWrites,
}

impl Default for SpendResetUnitOfWork {
    fn default() -> Self {
        let queue = Queue::default();
        Self {
            keys: KeySpendResetWrites {
                queue: Queue::clone(&queue),
            },
            users: UserSpendResetWrites {
                queue: Queue::clone(&queue),
            },
            teams: TeamSpendResetWrites { queue },
        }
    }
}

#[derive(Debug)]
pub struct BudgetCascadeUnitOfWork {
    pub team_memberships: LinkedSpendResetWrites,
    pub keys: KeyLinkedSpendResetWrites,
    pub organizations: LinkedSpendResetWrites,
    pub tags: LinkedSpendResetWrites,
    pub model_access_groups: LinkedSpendResetWrites,
    pub projects: LinkedSpendResetWrites,
    pub endusers: LinkedSpendResetWrites,
    pub budgets: BudgetWindowWrites,
}

impl Default for BudgetCascadeUnitOfWork {
    fn default() -> Self {
        let queue = Queue::default();
        Self {
            team_memberships: LinkedSpendResetWrites {
                table: "LiteLLM_TeamMembership",
                queue: Queue::clone(&queue),
            },
            keys: KeyLinkedSpendResetWrites {
                table: "LiteLLM_VerificationToken",
                queue: Queue::clone(&queue),
            },
            organizations: LinkedSpendResetWrites {
                table: "LiteLLM_OrganizationTable",
                queue: Queue::clone(&queue),
            },
            tags: LinkedSpendResetWrites {
                table: "LiteLLM_TagTable",
                queue: Queue::clone(&queue),
            },
            model_access_groups: LinkedSpendResetWrites {
                table: "LiteLLM_ModelAccessGroupBudgetTable",
                queue: Queue::clone(&queue),
            },
            projects: LinkedSpendResetWrites {
                table: "LiteLLM_ProjectTable",
                queue: Queue::clone(&queue),
            },
            endusers: LinkedSpendResetWrites {
                table: "LiteLLM_EndUserTable",
                queue: Queue::clone(&queue),
            },
            budgets: BudgetWindowWrites { queue },
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UnitOfWorkError<AppError>
where
    AppError: Error + 'static,
{
    #[error(transparent)]
    Body(AppError),
    #[error(transparent)]
    Commit(#[from] CommitError),
}

async fn run_unit_of_work<Unit, Body, Commit, CommitFut, Outcome, AppError>(
    mut unit: Unit,
    body: Body,
    commit: Commit,
) -> Result<Outcome, UnitOfWorkError<AppError>>
where
    Body: AsyncFnOnce(&mut Unit) -> Result<Outcome, AppError>,
    Commit: FnOnce(Unit) -> CommitFut,
    CommitFut: Future<Output = Result<(), CommitError>>,
    AppError: Error + 'static,
{
    let outcome = body(&mut unit).await.map_err(UnitOfWorkError::Body)?;
    commit(unit).await?;
    Ok(outcome)
}

macro_rules! unit_of_work_entry {
    ($name:ident, $batch:ty, $unit:ty) => {
        pub async fn $name<NewBatch, Body, Outcome, AppError>(
            new_batch: NewBatch,
            body: Body,
        ) -> Result<Outcome, UnitOfWorkError<AppError>>
        where
            NewBatch: FnOnce() -> $batch,
            Body: AsyncFnOnce(&mut $unit) -> Result<Outcome, AppError>,
            AppError: Error + 'static,
        {
            let batch = new_batch();
            run_unit_of_work(<$unit>::default(), body, |unit| async move {
                batch.commit(unit).await
            })
            .await
        }
    };
}

unit_of_work_entry!(
    spend_reset_unit_of_work,
    SpendResetBatch,
    SpendResetUnitOfWork
);
unit_of_work_entry!(
    budget_cascade_unit_of_work,
    BudgetCascadeBatch,
    BudgetCascadeUnitOfWork
);
