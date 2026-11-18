use betterlite_env::{EnvError, number};
use std::future::Future;

use super::chunk_outcome::ChunkOutcome;
use crate::database_urls::environment::EnvironmentSource;

pub const BATCH_SIZE_ENV: &str = "RESET_BUDGET_JOB_BATCH_SIZE";
pub const MAX_CHUNKS_ENV: &str = "RESET_BUDGET_JOB_MAX_CHUNKS_PER_RUN";
pub const DEFAULT_BATCH_SIZE: usize = 500;
pub const DEFAULT_MAX_CHUNKS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResetLimits {
    pub batch_size: usize,
    pub max_chunks_per_run: usize,
}

impl Default for ResetLimits {
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_BATCH_SIZE,
            max_chunks_per_run: DEFAULT_MAX_CHUNKS,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResetLimitsError {
    #[error(transparent)]
    Environment(#[from] EnvError),
}

impl ResetLimits {
    pub fn from_environment<S>(environment: &S) -> Result<Self, ResetLimitsError>
    where
        S: EnvironmentSource,
    {
        let batch_size = limit(
            environment.lookup(BATCH_SIZE_ENV).as_deref(),
            BATCH_SIZE_ENV,
            DEFAULT_BATCH_SIZE,
        )?;
        let max_chunks_per_run = limit(
            environment.lookup(MAX_CHUNKS_ENV).as_deref(),
            MAX_CHUNKS_ENV,
            DEFAULT_MAX_CHUNKS,
        )?;
        Ok(Self {
            batch_size,
            max_chunks_per_run,
        })
    }
}

// max(1, int(...)): LiteLLM clamps a configured zero or negative limit to one instead of failing.
fn limit(configured: Option<&str>, name: &str, default: usize) -> Result<usize, ResetLimitsError> {
    let Some(text) = configured else {
        return Ok(default);
    };
    let Some(value) = number::<i64>(name, text)? else {
        return Ok(default);
    };
    Ok(usize::try_from(value.max(1)).unwrap_or(usize::MAX))
}

// A short chunk means the due rows ran out. A full chunk that advanced nothing would be re-read unchanged forever, so it ends the phase too.
#[must_use]
pub const fn phase_is_drained(outcome: ChunkOutcome, batch_size: usize) -> bool {
    if outcome.fetched < batch_size {
        return true;
    }
    outcome.advanced == 0
}

pub async fn run_phase_in_chunks<Process, Progress>(limits: ResetLimits, mut process_chunk: Process)
where
    Process: FnMut() -> Progress + Send,
    Progress: Future<Output = ChunkOutcome> + Send,
{
    for _ in 0..limits.max_chunks_per_run {
        let outcome = process_chunk().await;
        if phase_is_drained(outcome, limits.batch_size) {
            return;
        }
    }
}

mod tests;
