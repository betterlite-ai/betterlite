use futures_util::stream::BoxStream;

use super::error::{LegacyPredictionError, PredictionError};
use super::projection::ProjectionServices;
use super::records::LegacyChunk;

mod completion;
pub mod completion_entry;
mod scope;
mod streaming;
pub mod streaming_entry;

pub type LegacyChunkStream = BoxStream<'static, Result<LegacyChunk, LegacyPredictionError>>;

pub struct LegacyContext<'config, 'encoding> {
    pub project: Option<&'config str>,
    pub location: Option<&'config str>,
    pub model_is_language: bool,
    pub services: &'config ProjectionServices<'encoding>,
}

pub(super) enum StreamFailure {
    Unable,
    Prediction(PredictionError),
}

impl From<PredictionError> for StreamFailure {
    fn from(source: PredictionError) -> Self {
        Self::Prediction(source)
    }
}
