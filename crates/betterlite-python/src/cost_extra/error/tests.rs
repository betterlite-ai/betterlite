#![cfg(test)]

use std::ffi::CStr;

use betterlite_core::cost::batch::error::BatchCostError;
use betterlite_core::cost::completion::error::ChargeInputError;
use betterlite_core::cost::realtime::error::RealtimeCostError;
use betterlite_models::duration_cost::error::DurationCostError;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::resolution::ResolutionError;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use rust_decimal::Decimal;

use crate::cost::convert::Amount;
use crate::cost_extra::convert::TranscriptionRequest;
use crate::cost_extra::error::{
    RealtimeFailure, RerankFailure, batch_error, image_error, realtime_error, rerank_error,
    resolution_error, transcription_error, video_error,
};

const NATIVE_FAKE: &CStr = c"class NativeError(Exception):\n    pass\n";

const EXCEPTIONS_FAKE: &CStr = c"class ModelNotMappedError(Exception):\n    pass\n\nclass BadRequestError(Exception):\n    def __init__(self, message: str, model: str, llm_provider: str) -> None:\n        self.message = f\"litellm.BadRequestError: {message}\"\n        self.model = model\n        self.llm_provider = llm_provider\n        super().__init__(self.message)\n";

// betterlite._native and betterlite.exceptions, so the raised class and its text can be read back.
fn install_fakes(py: Python<'_>) {
    let modules = py
        .import("sys")
        .expect("sys imports")
        .getattr("modules")
        .expect("sys.modules reads");
    let package = PyModule::new(py, "betterlite").expect("the package module is made");
    let native = PyModule::from_code(py, NATIVE_FAKE, c"fake_native.py", c"betterlite._native")
        .expect("the native fake is made");
    let exceptions = PyModule::from_code(
        py,
        EXCEPTIONS_FAKE,
        c"fake_exceptions.py",
        c"betterlite.exceptions",
    )
    .expect("the exceptions fake is made");
    package
        .setattr("_native", &native)
        .expect("the native fake is set");
    package
        .setattr("exceptions", &exceptions)
        .expect("the exceptions fake is set");
    modules
        .set_item("betterlite", &package)
        .expect("the package registers");
    modules
        .set_item("betterlite._native", &native)
        .expect("the native fake registers");
    modules
        .set_item("betterlite.exceptions", &exceptions)
        .expect("the exceptions fake registers");
}

fn missing_provider(model: &str) -> ResolutionError {
    ResolutionError::MissingProvider {
        model: model.to_owned(),
    }
}

#[test]
fn cost_extra_error_names_the_missing_provider() {
    Python::initialize();
    Python::attach(|py| {
        install_fakes(py);
        let error = resolution_error(py, &missing_provider("unknown-model"), "unknown-model");
        assert_eq!(
            error.to_string(),
            "BadRequestError: litellm.BadRequestError: LLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed model=unknown-model\n Pass model as E.g. For 'Huggingface' inference endpoints pass in `completion(model='huggingface/starcoder',..)` Learn more: https://docs.litellm.ai/docs/providers"
        );
    });
}

#[test]
fn cost_extra_error_keeps_the_endpoint_resolution_text() {
    Python::initialize();
    Python::attach(|py| {
        install_fakes(py);
        let error = resolution_error(
            py,
            &ResolutionError::EndpointWithoutProvider {
                endpoint: "endpoint-a".to_owned(),
                api_base: "https://endpoint-a.example".to_owned(),
            },
            "some-model",
        );
        assert!(
            error.to_string().contains(
                "api_base https://endpoint-a.example matches the registered endpoint endpoint-a, which names no provider"
            ),
            "{error}"
        );
    });
}

#[test]
fn cost_extra_error_maps_image_sizes_and_unknown_models() {
    Python::initialize();
    Python::attach(|py| {
        install_fakes(py);
        let size = image_error(
            py,
            &ImageCostError::MalformedSize {
                size: "100".to_owned(),
            },
        );
        assert!(size.is_instance_of::<PyValueError>(py), "{size}");
        let unknown = image_error(
            py,
            &ImageCostError::UnknownModel {
                model: "img".to_owned(),
            },
        );
        assert!(
            unknown
                .to_string()
                .contains("model img has no cost map entry")
        );
        assert!(unknown.to_string().contains("\"type\":\"Exception\""));
    });
}

#[test]
fn cost_extra_error_names_the_video_model_it_could_not_price() {
    Python::initialize();
    Python::attach(|py| {
        install_fakes(py);
        let unknown = video_error(
            py,
            &DurationCostError::UnknownModel {
                model: "veo".to_owned(),
            },
        );
        assert!(
            unknown
                .to_string()
                .contains("Model not found in cost map for model=veo")
        );
        let other = video_error(
            py,
            &DurationCostError::NoCharacterOrTokenPricing {
                key: "veo".to_owned(),
            },
        );
        assert!(
            other
                .to_string()
                .contains("does not have input_cost_per_character or input_cost_per_token pricing")
        );
    });
}

#[test]
fn cost_extra_error_raises_the_transcription_model_not_mapped_class() {
    Python::initialize();
    Python::attach(|py| {
        install_fakes(py);
        let request = TranscriptionRequest {
            model: "unknown-whisper".to_owned(),
            custom_llm_provider: Some("openai".to_owned()),
            duration: Amount(Decimal::ZERO),
        };
        let unknown = transcription_error(
            py,
            &DurationCostError::UnknownModel {
                model: "unknown-whisper".to_owned(),
            },
            &request,
        );
        assert!(
            unknown.to_string().starts_with(
                "ModelNotMappedError: This model isn't mapped yet. model=unknown-whisper, custom_llm_provider=openai."
            ),
            "{unknown}"
        );
        let other = transcription_error(py, &DurationCostError::Overflow, &request);
        assert!(
            other
                .to_string()
                .contains("duration cost overflowed Decimal arithmetic")
        );
    });
}

#[test]
fn cost_extra_error_maps_batch_resolution_and_overflow() {
    Python::initialize();
    Python::attach(|py| {
        install_fakes(py);
        let resolution = batch_error(
            py,
            &BatchCostError::Resolution {
                source: missing_provider("batch-model"),
            },
            "batch-model",
        );
        assert!(
            resolution.to_string().starts_with("BadRequestError:"),
            "{resolution}"
        );
        let overflow = batch_error(py, &BatchCostError::Overflow, "batch-model");
        assert!(
            overflow
                .to_string()
                .contains("batch cost arithmetic overflowed")
        );
    });
}

#[test]
fn cost_extra_error_maps_rerank_resolution_and_overflow() {
    Python::initialize();
    Python::attach(|py| {
        install_fakes(py);
        let resolution = rerank_error(
            py,
            &RerankFailure::Resolution {
                source: missing_provider("rerank-model"),
            },
            "rerank-model",
        );
        assert!(
            resolution.to_string().starts_with("BadRequestError:"),
            "{resolution}"
        );
        let overflow = rerank_error(py, &RerankFailure::Overflow, "rerank-model");
        assert!(
            overflow
                .to_string()
                .contains("the rerank cost arithmetic overflowed")
        );
    });
}

#[test]
fn cost_extra_error_maps_realtime_failures() {
    Python::initialize();
    Python::attach(|py| {
        install_fakes(py);
        let missing = realtime_error(
            py,
            &RealtimeFailure::Cost(RealtimeCostError::MissingField { field: "usage" }),
        );
        assert!(
            missing
                .to_string()
                .contains("the realtime event has no usage field")
        );
        let clock = realtime_error(
            py,
            &RealtimeFailure::Clock(ChargeInputError::CostHeader("x".to_owned())),
        );
        assert!(
            clock
                .to_string()
                .contains("the response cost header is not a number or a numeric string: x")
        );
    });
}
