use std::io::Cursor;

use polars::prelude::{DataFrame, ParquetCompression, ParquetWriter};

use super::{FocusSerializer, SerializationError};

pub struct FocusParquetSerializer;

impl FocusSerializer for FocusParquetSerializer {
    const EXTENSION: &'static str = "parquet";

    fn serialize(&self, frame: &DataFrame) -> Result<Vec<u8>> {
        let mut bytes = Cursor::new(Vec::new());
        ParquetWriter::new(&mut bytes)
            .with_compression(ParquetCompression::Snappy)
            .finish(&mut frame.clone())?;
        Ok(bytes.into_inner())
    }
}
