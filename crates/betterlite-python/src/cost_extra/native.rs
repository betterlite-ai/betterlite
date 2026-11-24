use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use rust_decimal::Decimal;

use crate::cost::convert::{Amount, encode, parse};
use crate::cost::error::{native_error, payload};
use crate::cost::settings::{Settings, SettingsWire};
use crate::cost_extra::convert::usage::UsageView;
use crate::cost_extra::convert::{
    BatchRequest, CollectUsageRequest, CombineUsageRequest, CostAnswer, ImageRequest,
    OcrBatchRequest, OcrCostRequest, PartitionUsageRequest, RealtimeCostAnswer, RealtimeCostWire,
    RealtimeTranscriptionWire, RerankRequest, TranscriptionRequest, VectorStoreRequest,
    VideoRequest,
};
use crate::cost_extra::engine::Engine;
use crate::cost_extra::engine::support::{collect_usage, combine_usage, partition_usage};
use crate::cost_extra::error::{
    batch_error, image_error, realtime_error, rerank_error, resolution_error, transcription_error,
    video_error,
};

// The extra-cost family's one entry point: settings text in, one request object as JSON per call.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeCostExtra {
    engine: Engine,
}

#[pymethods]
impl NativeCostExtra {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let wire: SettingsWire = parse(settings)?;
        Ok(Self {
            engine: Engine {
                settings: Settings::build(wire)?,
            },
        })
    }

    // LiteLLM's pair of floats with the whole OCR charge in the first slot.
    fn ocr_cost(&self, request: &str) -> PyResult<String> {
        let wire: OcrCostRequest = parse(request)?;
        let cost = self
            .engine
            .ocr_cost(&wire)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        encode(&[Amount(cost), Amount(Decimal::ZERO)]).map_err(PyErr::from)
    }

    fn ocr_batch_cost(&self, request: &str) -> PyResult<String> {
        let wire: OcrBatchRequest = parse(request)?;
        let cost = self
            .engine
            .ocr_batch_cost(&wire)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        encode(&[Amount(cost), Amount(Decimal::ZERO)]).map_err(PyErr::from)
    }

    fn vector_store_cost(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        let wire: VectorStoreRequest = parse(request)?;
        let (prompt, completion) = self.engine.vector_store_cost(&wire).map_err(|error| {
            resolution_error(py, &error, wire.model.as_deref().unwrap_or_default())
        })?;
        encode(&[Amount(prompt), Amount(completion)]).map_err(PyErr::from)
    }

    fn rerank_cost(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        let wire: RerankRequest = parse(request)?;
        let (prompt, completion) = self
            .engine
            .rerank_cost(&wire)
            .map_err(|error| rerank_error(py, &error, &wire.model))?;
        encode(&[Amount(prompt), Amount(completion)]).map_err(PyErr::from)
    }

    fn transcription_cost(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        let wire: TranscriptionRequest = parse(request)?;
        let (prompt, completion) = self.engine.transcription_cost(&wire).map_err(|error| {
            transcription_error(py, &error, &wire.model, wire.custom_llm_provider.as_deref())
        })?;
        encode(&[Amount(prompt), Amount(completion)]).map_err(PyErr::from)
    }

    fn image_cost(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        let wire: ImageRequest = parse(request)?;
        let cost = self
            .engine
            .image_cost(&wire)
            .map_err(|error| image_error(py, &error))?;
        encode(&CostAnswer { cost: Amount(cost) }).map_err(PyErr::from)
    }

    fn video_cost(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        let wire: VideoRequest = parse(request)?;
        let cost = self
            .engine
            .video_cost(&wire)
            .map_err(|error| video_error(py, &error))?;
        encode(&CostAnswer { cost: Amount(cost) }).map_err(PyErr::from)
    }

    fn batch_cost(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        let wire: BatchRequest = parse(request)?;
        let (prompt, completion) = self
            .engine
            .batch_cost(&wire)
            .map_err(|error| batch_error(py, &error, &wire.model))?;
        encode(&[Amount(prompt), Amount(completion)]).map_err(PyErr::from)
    }

    #[staticmethod]
    fn combine_usage(py: Python<'_>, request: &str) -> PyResult<String> {
        let wire: CombineUsageRequest = parse(request)?;
        let usage =
            combine_usage(&wire).map_err(|error| native_error(py, payload(error.to_string())))?;
        encode(&UsageView::from(&usage)).map_err(PyErr::from)
    }

    #[staticmethod]
    fn collect_usage(py: Python<'_>, request: &str) -> PyResult<String> {
        let wire: CollectUsageRequest = parse(request)?;
        let usages =
            collect_usage(&wire).map_err(|error| native_error(py, payload(error.to_string())))?;
        let views: Vec<UsageView> = usages.iter().map(UsageView::from).collect();
        encode(&views).map_err(PyErr::from)
    }

    #[staticmethod]
    fn partition_usage(py: Python<'_>, request: &str) -> PyResult<String> {
        let wire: PartitionUsageRequest = parse(request)?;
        let tiers =
            partition_usage(&wire).map_err(|error| native_error(py, payload(error.to_string())))?;
        encode(&tiers).map_err(PyErr::from)
    }

    fn realtime_cost(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        let wire: RealtimeCostWire = parse(request)?;
        let (cost, breakdown) = self
            .engine
            .realtime_cost(&wire)
            .map_err(|error| realtime_error(py, &error))?;
        let answer = RealtimeCostAnswer {
            cost: Amount(cost),
            breakdown: breakdown.as_ref(),
        };
        encode(&answer).map_err(PyErr::from)
    }

    fn realtime_transcription_cost(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        let wire: RealtimeTranscriptionWire = parse(request)?;
        let cost = self
            .engine
            .realtime_transcription_cost(&wire)
            .map_err(|error| native_error(py, payload(error.to_string())))?;
        encode(&Amount(cost)).map_err(PyErr::from)
    }
}
