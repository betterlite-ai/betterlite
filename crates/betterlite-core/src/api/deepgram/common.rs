use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use thiserror::Error;

// one provider exception class covers every Deepgram failure, and B1543 builds it for transcription
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{message}")]
pub struct DeepgramException {
    pub status_code: u16,
    pub message: String,
    pub headers: BTreeMap<String, Json>,
}
