use std::time::Duration;

use thiserror::Error;

use super::timeouts::DEFAULT_LOCK_TIMEOUT;

pub const MAX_MIGRATE_DEPLOY_ATTEMPTS: i32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrateAttemptBudget {
    pub attempts_left: i32,
    // elapsed Prisma-lock contention this attempt may still spend, measured monotonically
    pub contention_left: Duration,
}

#[derive(Debug, Error)]
pub enum BudgetError {
    #[error(
        "Timed out waiting for Prisma's migration advisory lock. Check the running migration or increase LITELLM_MIGRATION_LOCK_TIMEOUT."
    )]
    ContentionExhausted,
}

impl Default for MigrateAttemptBudget {
    fn default() -> Self {
        Self {
            attempts_left: MAX_MIGRATE_DEPLOY_ATTEMPTS,
            contention_left: DEFAULT_LOCK_TIMEOUT,
        }
    }
}

impl MigrateAttemptBudget {
    #[must_use]
    pub fn attempt_number(&self) -> i64 {
        i64::from(MAX_MIGRATE_DEPLOY_ATTEMPTS)
            .saturating_sub(i64::from(self.attempts_left))
            .saturating_add(1)
    }

    #[must_use]
    pub const fn spend(&self) -> Self {
        Self {
            // Python's int has no floor, so spending past zero keeps counting down; only i32::MIN wraps
            attempts_left: self.attempts_left.wrapping_sub(1),
            contention_left: self.contention_left,
        }
    }

    pub fn after_contention(&self, elapsed: Duration) -> Result<Self, BudgetError> {
        let remaining = self
            .contention_left
            .checked_sub(elapsed)
            .filter(|left| !left.is_zero())
            .ok_or(BudgetError::ContentionExhausted)?;
        Ok(Self {
            attempts_left: self.attempts_left,
            contention_left: remaining,
        })
    }
}
