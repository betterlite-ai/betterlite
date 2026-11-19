use std::error::Error;

use async_trait::async_trait;
use betterlite_compression::compress::CompressionSkippedReason;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::routes::CallType;

// The source passes these to compress() as keyword arguments for one call.
#[derive(Clone, Copy, Debug)]
pub struct CompressionCall<'call> {
    pub model: &'call str,
    pub call_type: CallType,
    pub messages: &'call [Json],
    pub compression_trigger: usize,
    pub compression_target: Option<usize>,
    pub embedding_model: Option<&'call str>,
    pub embedding_model_params: Option<&'call IndexMap<String, Json>>,
}

// The source reads compress's result dict field by field, so the token counts stay as sent.
#[derive(Clone, Debug, PartialEq)]
pub struct CompressionOutput {
    pub messages: Vec<Json>,
    pub tools: Vec<Json>,
    pub original_tokens: Json,
    pub compressed_tokens: Json,
    pub cache: IndexMap<String, String>,
    pub skipped_reason: Option<CompressionSkippedReason>,
}

#[async_trait]
pub trait CompressionEngine: Send + Sync {
    type Failure: Error + Send + Sync + 'static;

    async fn compress(
        &self,
        call: &CompressionCall<'_>,
    ) -> Result<CompressionOutput, Self::Failure>;
}
