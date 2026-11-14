pub mod error;
mod estimate;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_roi::estimator::models::{EstimatorModel, ROICompletionRequest};
use betterlite_roi::estimator::options::configured_models;
use betterlite_roi::settings::ROISettings;
use serde_json::value::RawValue;

use crate::roi_github::error::SourceError;

pub type CompletionObject = Box<RawValue>;
pub type CompletionFuture =
    Pin<Box<dyn Future<Output = Result<CompletionObject, SourceError>> + Send + 'static>>;
pub type CompletionCaller = Arc<dyn Fn(ROICompletionRequest) -> CompletionFuture + Send + Sync>;

pub struct Estimator {
    pub settings: ROISettings,
    pub complete: CompletionCaller,
    pub models: Vec<EstimatorModel>,
}

impl Estimator {
    pub fn new(
        settings: ROISettings,
        complete: CompletionCaller,
        models: Option<&[EstimatorModel]>,
    ) -> Self {
        Self {
            models: configured_models(&settings, models),
            settings,
            complete,
        }
    }
}

include!("roi_estimator/tests/declarations.rs");
