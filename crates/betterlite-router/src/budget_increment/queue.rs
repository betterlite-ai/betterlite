use std::mem::take;
use std::time::Duration;

use rust_decimal::Decimal;

use super::error::IncrementError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueuedIncrement {
    pub key: String,
    pub increment_value: Decimal,
    pub ttl: Duration,
}

#[derive(Debug, Default)]
pub struct IncrementQueue {
    pub queued: Vec<QueuedIncrement>,
    pub detached: Option<Vec<QueuedIncrement>>,
}

impl IncrementQueue {
    pub fn detach(&mut self) -> Vec<QueuedIncrement> {
        if let Some(detached) = &self.detached {
            return detached.clone();
        }
        if self.queued.is_empty() {
            return Vec::new();
        }
        let batch = take(&mut self.queued);
        self.detached = Some(batch.clone());
        batch
    }

    // a merged overflow leaves the batch detached, so the next attempt writes it unmerged instead of losing it
    pub fn requeue(&mut self) -> Result<(), IncrementError> {
        let Some(detached) = &self.detached else {
            return Ok(());
        };
        let mut operations = detached
            .iter()
            .chain(self.queued.iter())
            .cloned()
            .collect::<Vec<_>>();
        operations.sort_by(|left, right| left.key.cmp(&right.key));
        let mut merged: Vec<QueuedIncrement> = Vec::with_capacity(operations.len());
        for operation in operations {
            match merged.last_mut() {
                Some(last) if last.key == operation.key => {
                    let total = last
                        .increment_value
                        .checked_add(operation.increment_value)
                        .ok_or_else(|| IncrementError::SpendOverflow {
                            key: operation.key.clone(),
                        })?;
                    last.increment_value = total;
                    last.ttl = operation.ttl;
                }
                _ => merged.push(operation),
            }
        }
        self.queued = merged;
        self.detached = None;
        Ok(())
    }
}
