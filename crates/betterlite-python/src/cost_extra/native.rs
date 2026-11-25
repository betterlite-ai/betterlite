use std::time::SystemTime;

use pyo3::{exceptions::PyValueError, prelude::*};
use serde::{Serialize, de::DeserializeOwned};

use crate::cost::convert::{UsageRecord, encode, parse};
use crate::cost::error::{native_error, payload};
use crate::cost::settings::Settings;
use crate::cost_extra::convert::usage::UsageWire;
use crate::cost_extra::convert::{CollectUsageRequest, CombineUsageRequest, PartitionUsageRequest};
use crate::cost_extra::engine::Engine;
use crate::cost_extra::engine::support::{collect_usage, combine_usage, partition_usage};
use crate::cost_extra::error::{
    batch_error, image_error, realtime_error, rerank_error, transcription_error, video_error,
};

mod tests;

#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeCostExtra {
    engine: Engine,
}

// One specialised call: the request parsed, the engine run, its failure mapped and the answer encoded.
fn answer<W, E, T, Run, Fail>(request: &str, run: Run, fail: Fail) -> PyResult<String>
where
    W: DeserializeOwned,
    T: Serialize,
    Run: FnOnce(&W) -> Result<T, E>,
    Fail: FnOnce(&W, &E) -> PyErr,
{
    let wire: W = parse(request)?;
    let value = run(&wire).map_err(|error| fail(&wire, &error))?;
    encode(&value).map_err(PyErr::from)
}

#[pymethods]
impl NativeCostExtra {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let engine = Engine {
            settings: Settings::build(parse(settings)?)?,
        };
        Ok(Self { engine })
    }

    fn ocr_cost(&self, request: &str) -> PyResult<String> {
        answer(
            request,
            |wire| self.engine.ocr_cost(wire),
            |_, error| PyValueError::new_err(error.to_string()),
        )
    }

    fn ocr_batch_cost(&self, request: &str) -> PyResult<String> {
        answer(
            request,
            |wire| self.engine.ocr_batch_cost(wire),
            |_, error| PyValueError::new_err(error.to_string()),
        )
    }

    fn vector_store_cost(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        answer(
            request,
            |wire| self.engine.vector_store_cost(wire),
            |wire, error| rerank_error(py, error, wire.model.as_deref().unwrap_or_default()),
        )
    }

    fn rerank_cost(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        answer(
            request,
            |wire| self.engine.rerank_cost(wire),
            |wire, error| rerank_error(py, error, &wire.model),
        )
    }

    fn transcription_cost(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        answer(
            request,
            |wire| self.engine.transcription_cost(wire),
            |wire, error| transcription_error(py, error, wire),
        )
    }

    fn image_cost(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        answer(
            request,
            |wire| self.engine.image_cost(wire),
            |_, error| image_error(py, error),
        )
    }

    fn video_cost(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        answer(
            request,
            |wire| self.engine.video_cost(wire),
            |_, error| video_error(py, error),
        )
    }

    fn batch_cost(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        answer(
            request,
            |wire| self.engine.batch_cost(wire),
            |wire, error| batch_error(py, error, &wire.model),
        )
    }

    #[staticmethod]
    fn combine_usage(py: Python<'_>, request: &str) -> PyResult<String> {
        let wire: CombineUsageRequest = parse(request)?;
        let usage =
            combine_usage(&wire).map_err(|error| native_error(py, payload(error.to_string())))?;
        encode(&UsageWire::from(&usage)).map_err(PyErr::from)
    }

    #[staticmethod]
    fn collect_usage(py: Python<'_>, request: &str) -> PyResult<String> {
        let wire: CollectUsageRequest = parse(request)?;
        let usages =
            collect_usage(&wire).map_err(|error| native_error(py, payload(error.to_string())))?;
        let views: Vec<UsageRecord<'_>> = usages.iter().map(UsageRecord::from).collect();
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
        let mut clock = SystemTime::now;
        answer(
            request,
            |wire| self.engine.realtime_cost(wire, &mut clock),
            |_, error| realtime_error(py, error),
        )
    }

    fn realtime_transcription_cost(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        answer(
            request,
            |wire| self.engine.realtime_transcription_cost(wire),
            |_, error| native_error(py, payload(error.to_string())),
        )
    }
}
