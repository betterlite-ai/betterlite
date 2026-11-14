use std::str::{Utf8Error, from_utf8};

use crate::migration_ledger::connection::ConnectionError;

pub mod cells;
pub mod fixtures;
pub mod migration;
pub mod unassigned;

use cells::{HeaderIndex, RowError, VerificationTokenRow};
use migration::ImportError;
use unassigned::UnassignedTeamError;

pub const MAX_CSV_BYTES: usize = 0x0010_0000;
// one byte past the cap, so a capped read and a file that fits look different
pub const MAX_CSV_READ_BYTES: u64 = 0x0010_0001;
pub const MAX_JSON_DEPTH: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("the verification-token CSV is {len} bytes, over the {MAX_CSV_BYTES} byte limit")]
    TooLarge { len: usize },
    #[error("the verification-token CSV is not valid UTF-8")]
    InvalidUtf8(#[source] Utf8Error),
    #[error("the verification-token CSV headers could not be read")]
    Headers(#[source] csv::Error),
    #[error("the verification-token CSV records could not be read")]
    Record(#[source] csv::Error),
}

#[derive(Debug)]
pub struct TypedValue {
    pub rows: Vec<TypedRow>,
}

#[derive(Debug)]
pub struct TypedRow {
    pub token: Option<String>,
    pub outcome: Result<VerificationTokenRow, RowError>,
}

pub fn decode_verification_csv(bytes: &[u8]) -> Result<TypedValue, DecodeError> {
    if bytes.len() > MAX_CSV_BYTES {
        return Err(DecodeError::TooLarge { len: bytes.len() });
    }
    let text = from_utf8(bytes).map_err(DecodeError::InvalidUtf8)?;
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers = reader.headers().map_err(DecodeError::Headers)?.clone();
    let columns = HeaderIndex::read(&headers);
    let mut rows = Vec::new();
    for record in reader.records() {
        let fields = record.map_err(DecodeError::Record)?;
        rows.push(cells::decode_row(&columns, &fields));
    }
    Ok(TypedValue { rows })
}

#[derive(Debug, thiserror::Error)]
pub enum RepairError {
    #[error("the repair database connection could not be opened: {0}")]
    Connection(#[source] Box<ConnectionError>),
    #[error("{0}")]
    Import(#[from] ImportError),
    #[error("{0}")]
    Teams(#[from] UnassignedTeamError),
}

mod tests;
