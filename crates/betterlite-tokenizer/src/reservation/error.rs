use thiserror::Error;
use tokio::task::JoinError;

use crate::count::error::CountError;

#[derive(Debug, Error)]
pub enum ReservationError {
    #[error("the messages value is not a list of messages")]
    Messages,
    #[error("the tools value is not a list")]
    Tools,
    #[error("the tool_choice value names no function")]
    ToolChoice,
    #[error("the input token count failed")]
    Count {
        #[source]
        source: Box<CountError>,
    },
    #[error("the token count does not fit in a machine word")]
    Overflow,
    #[error("the reservation accounting pool is closed")]
    PoolClosed,
    #[error("the reservation accounting worker did not run")]
    WorkerMissing,
    #[error("the reservation accounting worker failed")]
    Worker {
        #[source]
        source: JoinError,
    },
}

impl From<CountError> for ReservationError {
    fn from(source: CountError) -> Self {
        Self::Count {
            source: Box::new(source),
        }
    }
}
