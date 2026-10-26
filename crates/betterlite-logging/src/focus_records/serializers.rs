pub mod csv;
pub mod parquet;

use polars::prelude::DataFrame;

pub trait FocusSerializer {
    const EXTENSION: &'static str;

    fn serialize(&self, frame: &DataFrame) -> Result<Vec<u8>, polars::error::PolarsError>;
}
