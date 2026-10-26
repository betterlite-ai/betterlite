pub mod csv;
pub mod parquet;

use polars::prelude::{DataFrame, PolarsError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SerializationError {
    #[error("FOCUS serialization failed")]
    Polars(#[from] PolarsError),
}

pub trait FocusSerializer {
    const EXTENSION: &'static str;

    fn serialize(&self, frame: &DataFrame) -> Result<Vec<u8>, SerializationError>;
}
