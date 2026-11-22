use thiserror::Error;

use super::parse::MAX_DEPTH;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RecordJsonError {
    #[error("the line is not in an encoding json.loads reads")]
    Encoding,
    #[error("the line is not valid JSON")]
    Syntax,
    #[error("the line nests deeper than {MAX_DEPTH} levels")]
    Depth,
    #[error("the line is not a JSON object")]
    NotObject,
}
