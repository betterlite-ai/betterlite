#![cfg(test)]

use std::sync::{Mutex, PoisonError};

use crate::redis_spend::log_row::SpendLogRow;
use crate::spend_log_storage::batch_claim::{BatchChargeRow, BatchClaimTable};
use crate::spend_log_storage::{SpendLogFuture, SpendLogWriteError, SpendLogsWriter};

#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("the scripted spend log store failed")]
pub(crate) struct ClaimFailure;

impl SpendLogWriteError for ClaimFailure {
    fn is_transient(&self) -> bool {
        false
    }

    fn is_data_rejection(&self) -> bool {
        false
    }

    fn is_service_unavailable(&self) -> bool {
        false
    }

    fn is_deadlock(&self) -> bool {
        false
    }

    fn is_database(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InsertOutcome {
    Inserted,
    Duplicate,
    Failed,
}

// The claim store a case scripts: what create_many answers, the row find_unique reads back, and how many rows update_many writes.
pub(crate) struct ScriptedClaim {
    pub(crate) insert: InsertOutcome,
    pub(crate) existing: Option<BatchChargeRow>,
    pub(crate) taken_over: usize,
    pub(crate) rows: Mutex<Vec<SpendLogRow>>,
}

impl ScriptedClaim {
    pub(crate) const fn answering(outcome: InsertOutcome) -> Self {
        Self {
            insert: outcome,
            existing: None,
            taken_over: 0,
            rows: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn with_existing(mut self, existing: BatchChargeRow) -> Self {
        self.existing = Some(existing);
        self
    }

    pub(crate) fn rows(&self) -> Vec<SpendLogRow> {
        self.rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SpendLogsWriter for ScriptedClaim {
    type Error = ClaimFailure;

    fn insert_rows<'call>(
        &'call self,
        rows: &'call [SpendLogRow],
    ) -> SpendLogFuture<'call, Result<usize, ClaimFailure>> {
        let outcome = self.insert;
        Box::pin(async move {
            if outcome == InsertOutcome::Failed {
                return Err(ClaimFailure);
            }
            self.rows
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend_from_slice(rows);
            if outcome == InsertOutcome::Inserted {
                return Ok(1);
            }
            Ok(0)
        })
    }
}

impl BatchClaimTable for ScriptedClaim {
    fn find_batch_row<'call>(
        &'call self,
        _request_id: &'call str,
    ) -> SpendLogFuture<'call, Result<Option<BatchChargeRow>, ClaimFailure>> {
        Box::pin(async move { Ok(self.existing.clone()) })
    }

    fn take_over<'call>(
        &'call self,
        _data: &'call SpendLogRow,
        _request_id: &'call str,
    ) -> SpendLogFuture<'call, Result<usize, ClaimFailure>> {
        Box::pin(async move { Ok(self.taken_over) })
    }
}
