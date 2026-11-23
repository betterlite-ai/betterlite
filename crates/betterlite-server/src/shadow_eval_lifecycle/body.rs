use thiserror::Error;

use crate::shadow_eval_validation::error::ShadowEvalRequestError;
use crate::shadow_eval_validation::request::{StartShadowEvalRequest, parse_shadow_eval_request};

// the shadow_body outside-bytes bound: one mebibyte, checked before any decode
pub const SHADOW_BODY_MAX_BYTES: usize = 1_048_576;

#[derive(Debug, Error)]
pub enum ShadowBodyError {
    #[error("the shadow eval request is {bytes} bytes, over the {limit} byte limit")]
    TooLarge { bytes: usize, limit: usize },
    #[error(transparent)]
    Request(#[from] ShadowEvalRequestError),
}

// the production bounded parser every shadow body enters through: the size gate runs first, then B345's field parser
pub fn decode_shadow_body(bytes: &[u8]) -> Result<StartShadowEvalRequest, ShadowBodyError> {
    if bytes.len() > SHADOW_BODY_MAX_BYTES {
        return Err(ShadowBodyError::TooLarge {
            bytes: bytes.len(),
            limit: SHADOW_BODY_MAX_BYTES,
        });
    }
    parse_shadow_eval_request(bytes).map_err(ShadowBodyError::Request)
}
