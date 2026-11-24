use std::time::SystemTime;

use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::get_runtime;

use crate::cost::billing::{CallRequest, CompletionRequest};
use crate::cost::breakdown::Capture;
use crate::cost::convert::{
    Amount, CostAnswer, RatesView, UsageAnswer, UsageRecord, UsageRequest, encode, parse,
};
use crate::cost::engine::{Engine, usage_object as usage_from_request};
use crate::cost::error::{charge_error, native_error, payload, priced_error};
use crate::cost::settings::{Settings, SettingsWire};

mod tests;

// The cost family's one entry point: settings text in, one request object as JSON per call.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeCost {
    engine: Engine,
}

#[pymethods]
impl NativeCost {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let wire: SettingsWire = parse(settings)?;
        let built = Settings::build(wire)?;
        Ok(Self {
            engine: Engine { settings: built },
        })
    }

    // LiteLLM's pair of floats, in the order cost_per_token returns them.
    fn cost_per_token(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        let wire: CallRequest = parse(request)?;
        let model = wire.model.as_deref().unwrap_or_default();
        let provider = wire.custom_llm_provider.as_deref();
        let (prompt, completion) = self
            .engine
            .cost_per_token(&wire)
            .map_err(|error| priced_error(py, &error, model, provider))?;
        encode(&[Amount(prompt), Amount(completion)]).map_err(PyErr::from)
    }

    // ast-grep-ignore: no-one-line-functions
    fn completion_cost(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        priced_response(py, &self.engine, request)
    }

    // ast-grep-ignore: no-one-line-functions
    fn response_cost(&self, py: Python<'_>, request: &str) -> PyResult<String> {
        priced_response(py, &self.engine, request)
    }

    // LiteLLM's response mapping for get_usage_object: the usage, or JSON null when it carries none.
    #[staticmethod]
    fn usage_object(py: Python<'_>, request: &str) -> PyResult<String> {
        let wire: UsageRequest = parse(request)?;
        let usage = usage_from_request(&wire)
            .map_err(|error| native_error(py, payload(error.to_string())))?;
        encode(&UsageAnswer {
            usage: usage.as_ref().map(UsageRecord::from),
        })
        .map_err(PyErr::from)
    }
}

// One charge on the shared runtime with the GIL released, then the answer with its breakdown.
fn priced_response(py: Python<'_>, engine: &Engine, request: &str) -> PyResult<String> {
    let wire: CompletionRequest = parse(request)?;
    let answered = py.detach(|| {
        let mut clock = SystemTime::now;
        let mut capture = Capture::default();
        get_runtime()
            .block_on(engine.charge(&wire, &mut clock, Some(&mut capture)))
            .map(|cost| (cost, capture))
    });
    let (cost, capture) = answered.map_err(|error| charge_error(py, &error, &wire))?;
    let answer = CostAnswer {
        cost: Amount(cost),
        breakdown: capture.breakdown.as_ref(),
        billed_token_rates: capture.rates.map(RatesView::from),
    };
    encode(&answer).map_err(PyErr::from)
}
