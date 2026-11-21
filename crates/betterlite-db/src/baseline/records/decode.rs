use betterlite_models::baseline_history::model::BaselineHistory;

use super::{BaselineAccountingRecord, RecordDecodeError, decode_baseline_record};
use crate::baseline::publication::decode::{PublicationDecodeError, decode_baseline_publication};
use crate::baseline::publication::model::BaselinePublication;

// The three stored baseline texts, tagged by the leading marker byte a fuzz seed or a replay row carries.
#[derive(Clone)]
pub enum BaselineJson {
    Record(Box<BaselineAccountingRecord>),
    Publication(Box<BaselinePublication>),
    History(Box<BaselineHistory>),
}

#[derive(Debug, thiserror::Error)]
pub enum BaselineJsonError {
    #[error("stored baseline JSON must start with R, P or H")]
    UnknownKind,
    #[error(transparent)]
    Record(#[from] RecordDecodeError),
    #[error(transparent)]
    Publication(#[from] PublicationDecodeError),
    #[error(transparent)]
    History(#[from] serde_json::Error),
}

// One dispatch for the fuzz target and its corpus replay; a history row uses its dataclass's own coercions.
pub fn decode_baseline_json(data: &[u8]) -> Result<BaselineJson, BaselineJsonError> {
    let Some((&marker, rest)) = data.split_first() else {
        return Err(BaselineJsonError::UnknownKind);
    };
    match marker {
        b'R' => Ok(BaselineJson::Record(Box::new(decode_baseline_record(
            rest,
        )?))),
        b'P' => Ok(BaselineJson::Publication(Box::new(
            decode_baseline_publication(rest)?,
        ))),
        b'H' => Ok(BaselineJson::History(Box::new(serde_json::from_slice(
            rest,
        )?))),
        _ => Err(BaselineJsonError::UnknownKind),
    }
}
