use std::io;

use thiserror::Error;

use super::record_json::error::RecordJsonError;

#[derive(Debug, Error)]
pub enum BatchGuardrailError {
    #[error("reading the batch input file failed")]
    Read {
        #[source]
        source: io::Error,
    },
    #[error(
        "batch input line {line_number} could not be read as the JSON the upload validation accepted"
    )]
    Record {
        line_number: usize,
        #[source]
        source: RecordJsonError,
    },
}
