use std::io;
use std::num::{ParseFloatError, TryFromIntError};

use parquet2::error::Error as ParquetError;
use rust_decimal::Decimal;

use super::table::FocusTable;

mod csv;
mod parquet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusFormat {
    Csv,
    Parquet,
}

#[derive(Debug, thiserror::Error)]
pub enum SerializeError {
    #[error("Decimal {value} could not be written as a float")]
    Float {
        value: Decimal,
        #[source]
        source: ParseFloatError,
    },
    #[error("Decimal {value} does not fit a 64-bit Parquet decimal")]
    Decimal {
        value: Decimal,
        #[source]
        source: TryFromIntError,
    },
    #[error("a Parquet length or count does not fit its field")]
    Length(#[source] TryFromIntError),
    #[error("the Parquet definition levels could not be written")]
    Levels(#[source] io::Error),
    #[error("the Parquet writer failed")]
    Parquet(#[source] ParquetError),
}

impl FocusFormat {
    #[must_use = "format extension must be used"]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Parquet => "parquet",
        }
    }

    #[must_use = "serialized FOCUS content must be delivered or handled"]
    pub fn encode(self, table: &FocusTable) -> Result<Vec<u8>, SerializeError> {
        match self {
            Self::Csv => csv::encode(table),
            Self::Parquet => parquet::encode(table),
        }
    }
}

mod tests;
