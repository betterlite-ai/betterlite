use std::num::TryFromIntError;

use tokio::time::error::Elapsed;

use crate::autorouter_turn_sql::write::TurnWriteError;
use crate::baseline::publication::decode::PublicationDecodeError;
use crate::baseline::publication::project::ProjectionError;
use crate::baseline::publication::publish::PublishError;
use crate::baseline::records::RecordDecodeError;
use crate::postgres_store::errors::DbError;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("the baseline writer connection is unavailable")]
    Writer(#[from] DbError),
    #[error("a baseline accounting statement failed")]
    Statement(#[from] tokio_postgres::Error),
    #[error("the baseline transaction passed its client deadline")]
    Deadline(#[from] Elapsed),
    #[error("the observation's turn could not be written")]
    Turn(#[from] TurnWriteError),
    #[error("baseline corrections could not be published")]
    Publish(#[from] PublishError),
    #[error("baseline history could not be replayed")]
    Projection(#[from] ProjectionError),
    #[error("a stored baseline record cannot be decoded")]
    Record(#[from] RecordDecodeError),
    #[error("a stored baseline publication cannot be decoded")]
    Publication(#[from] PublicationDecodeError),
    #[error("the stored baseline history is not valid JSON")]
    History(#[from] serde_json::Error),
    #[error("the comparison revision cannot advance past the largest bigint")]
    Revision,
    #[error("the retirement batch size does not fit a PostgreSQL integer")]
    BatchSize(#[from] TryFromIntError),
}
