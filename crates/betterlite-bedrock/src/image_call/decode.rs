use serde::de::IgnoredAny;
use thiserror::Error;

use crate::image_models::family::Family;
use crate::image_models::response::{
    ImagesOnlyResponse, Stability1Response, Stability3Response, TitanResponse,
};

#[derive(Debug, Error)]
pub enum ReplyDecodeError {
    #[error("the Bedrock image reply is JSON null")]
    Null,
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyRecord {
    Titan(TitanResponse),
    Stability1(Stability1Response),
    Stability3(Stability3Response),
    Nova(ImagesOnlyResponse),
}

// the HTTP boundary parses provider JSON, and only then does a B633 record transform map the images
pub fn decode_bedrock_image_reply(
    bytes: &[u8],
    family: Family,
) -> Result<ReplyRecord, ReplyDecodeError> {
    let parsed = match family {
        Family::Titan => serde_json::from_slice(bytes).map(ReplyRecord::Titan),
        Family::Stability1 => serde_json::from_slice(bytes).map(ReplyRecord::Stability1),
        Family::Stability3 => serde_json::from_slice(bytes).map(ReplyRecord::Stability3),
        Family::Nova => serde_json::from_slice(bytes).map(ReplyRecord::Nova),
    };
    match parsed {
        Ok(record) => Ok(record),
        Err(source) => {
            // Python's response.json() answers None only for the null token, before any record constructor runs
            if serde_json::from_slice::<Option<IgnoredAny>>(bytes)
                .is_ok_and(|value| value.is_none())
            {
                return Err(ReplyDecodeError::Null);
            }
            Err(ReplyDecodeError::Json(source))
        }
    }
}
