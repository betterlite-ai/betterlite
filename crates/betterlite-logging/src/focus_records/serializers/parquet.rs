use std::io::Cursor;

use polars::prelude::{DataFrame, ParquetCompression, ParquetWriter, PolarsError};

use super::FocusSerializer;

pub struct FocusParquetSerializer;

impl FocusSerializer for FocusParquetSerializer {
    const EXTENSION: &'static str = "parquet";

    fn serialize(&self, frame: &DataFrame) -> Result<Vec<u8>, PolarsError> {
        let mut bytes = Cursor::new(Vec::new());
        ParquetWriter::new(&mut bytes)
            .with_compression(ParquetCompression::Snappy)
            .finish(&mut frame.clone())?;
        Ok(bytes.into_inner())
    }
}
