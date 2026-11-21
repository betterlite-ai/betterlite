use betterlite_models::baseline_history::model::BaselineHistory;

use super::{BaselineAccountingRecord, RecordDecodeError, decode_baseline_record};
use crate::baseline::publication::decode::{
    PublicationDecodeError, decode_baseline_history, decode_baseline_publication,
};
use crate::baseline::publication::model::BaselinePublication;

// The three stored baseline texts, tagged by the leading marker byte a fuzz seed or a replay row carries.
#[derive(Clone, Debug)]
pub enum BaselineJson {
    Record(BaselineAccountingRecord),
    Publication(BaselinePublication),
    History(BaselineHistory),
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

// One dispatch for the fuzz target and its corpus replay, so both exercise the production decoders.
pub fn decode_baseline_json(data: &[u8]) -> Result<BaselineJson, BaselineJsonError> {
    match data.split_first() {
        Some((b'R', rest)) => Ok(BaselineJson::Record(decode_baseline_record(rest)?)),
        Some((b'P', rest)) => Ok(BaselineJson::Publication(decode_baseline_publication(
            rest,
        )?)),
        Some((b'H', rest)) => Ok(BaselineJson::History(decode_baseline_history(rest)?)),
        Some((_, _)) => Err(BaselineJsonError::UnknownKind),
        None => Err(BaselineJsonError::UnknownKind),
    }
}
