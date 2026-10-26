use std::io::Cursor;

use polars::prelude::{CsvWriter, DataFrame, DataType, SchemaExt as _, SerWriter as _};

use super::{FocusSerializer, SerializationError};

pub struct FocusCsvSerializer;

impl FocusSerializer for FocusCsvSerializer {
    const EXTENSION: &'static str = "csv";

    fn serialize(&self, frame: &DataFrame) -> Result<Vec<u8>, SerializationError> {
        let decimal_columns: Vec<String> = frame
            .schema()
            .iter_fields()
            .filter(|field| field.dtype().is_decimal())
            .map(|field| field.name().to_string())
            .collect();
        let mut output = frame.clone();
        for name in decimal_columns {
            output.try_apply(name.as_str(), |column| column.cast(&DataType::Float64))?;
        }
        let mut bytes = Cursor::new(Vec::new());
        CsvWriter::new(&mut bytes).finish(&mut output)?;
        Ok(bytes.into_inner())
    }
}
