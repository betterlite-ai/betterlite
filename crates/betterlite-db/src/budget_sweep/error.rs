use std::error::Error;

use super::phase::ResetPhase;

#[derive(Debug, thiserror::Error)]
pub enum SweepError<Failure>
where
    Failure: Error + 'static,
{
    #[error("the {} budget reset phase failed", .phase.name())]
    Phase {
        phase: ResetPhase,
        #[source]
        source: Failure,
    },
}
