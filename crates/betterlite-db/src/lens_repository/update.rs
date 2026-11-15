use thiserror::Error;

use crate::lens_repository::repository::LensRepositoryError;

pub const DEFAULT_ATTEMPTS: u32 = 8;

#[derive(Debug, Error)]
pub enum LensUpdateError<E> {
    #[error(transparent)]
    Repository(#[from] LensRepositoryError),
    #[error("the lens update transform returned an error")]
    Transform(E),
}
