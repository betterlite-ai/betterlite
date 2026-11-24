#![cfg(test)]

use std::ffi::CStr;

use betterlite_core::cost::call_price::error::CallPriceError;
use betterlite_core::cost::completion::error::CompletionCostError;
use betterlite_core::cost::prepare::error::PrepareError;
use betterlite_core::cost::usage::BillingUsageError;
use betterlite_models::duration_cost::error::DurationCostError;
use betterlite_models::resolution::ResolutionError;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_tokenizer::count::error::CountError;
use pyo3::prelude::*;

use crate::cost::billing::CompletionRequest;
use crate::cost::convert::{ConvertError, parse};
use crate::cost::error::{EngineError, charge_error, native_error, payload, priced_error};
use crate::cost::settings::SettingsError;

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

fn completion_request(text: &str) -> CompletionRequest {
    parse(text).expect("the completion request parses")
}

fn json_error() -> serde_json::Error {
    serde_json::from_str::<CompletionRequest>("not json").expect_err("the text is not JSON")
}

#[test]
fn cost_priced_error_raises_model_not_mapped_for_a_model_without_a_price() {
    Python::initialize();
    Python::attach(|py| {
        install_fakes(py);
        let error = priced_error(
            py,
            &EngineError::Price(CallPriceError::Generic(TokenCostError::UnknownModel)),
            "unknown-model",
            Some("openai"),
        );
        assert_eq!(
            error.to_string(),
            "ModelNotMappedError: This model isn't mapped yet. model=unknown-model, custom_llm_provider=openai. Add it here - https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json."
        );
    });
}

#[test]
fn cost_priced_error_raises_bad_request_when_no_provider_resolves() {
    Python::initialize();
    Python::attach(|py| {
        install_fakes(py);
        let error = priced_error(
            py,
            &EngineError::Price(CallPriceError::Resolution {
                source: ResolutionError::MissingProvider {
                    model: "unknown-model".to_owned(),
                },
            }),
            "unknown-model",
            None,
        );
        assert_eq!(
            error.to_string(),
            "BadRequestError: litellm.BadRequestError: LLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed model=unknown-model\n Pass model as E.g. For 'Huggingface' inference endpoints pass in `completion(model='huggingface/starcoder',..)` Learn more: https://docs.litellm.ai/docs/providers"
        );
    });
}

#[test]
fn cost_priced_error_keeps_the_null_model_exception() {
    Python::initialize();
    Python::attach(|py| {
        install_fakes(py);
        let error = priced_error(py, &EngineError::Price(CallPriceError::NullModel), "", None);
        assert!(
            error
                .to_string()
                .contains("Invalid arg. Model cannot be none.")
        );
        assert!(error.to_string().contains("\"type\":\"Exception\""));
    });
}

#[test]
fn cost_priced_error_sees_a_price_failure_inside_a_completion_error() {
    Python::initialize();
    Python::attach(|py| {
        install_fakes(py);
        let error = priced_error(
            py,
            &EngineError::Completion(CompletionCostError::Price(CallPriceError::Generic(
                TokenCostError::UnknownModel,
            ))),
            "unknown-model",
            Some("openai"),
        );
        assert!(
            error
                .to_string()
                .starts_with("ModelNotMappedError: This model isn't mapped yet.")
        );
    });
}

#[test]
fn cost_charge_error_names_the_model_the_response_carries() {
    Python::initialize();
    Python::attach(|py| {
        install_fakes(py);
        let request = completion_request(
            r#"{"custom_llm_provider": "openai",
                "response": {"model": "unknown-model", "usage": {"prompt_tokens": 1}}}"#,
        );
        let error = charge_error(
            py,
            &EngineError::Price(CallPriceError::Duration(DurationCostError::UnknownModel {
                model: "duration-model".to_owned(),
            })),
            &request,
        );
        assert!(error.to_string().starts_with("ModelNotMappedError: This model isn't mapped yet. model=unknown-model, custom_llm_provider=openai."));
    });
}

#[test]
fn cost_charge_error_raises_a_value_error_without_a_model() {
    Python::initialize();
    Python::attach(|py| {
        install_fakes(py);
        let error = charge_error(
            py,
            &EngineError::Completion(CompletionCostError::Prepare(PrepareError::ModelMissing)),
            &completion_request("{}"),
        );
        assert_eq!(
            error.to_string(),
            "ValueError: Model is None and does not exist in passed completion_response. Passed completion_response=None, model=None"
        );
    });
}

#[test]
fn cost_charge_error_explains_the_uncounted_text() {
    Python::initialize();
    Python::attach(|py| {
        install_fakes(py);
        let error = charge_error(
            py,
            &EngineError::Completion(CompletionCostError::Prepare(PrepareError::Counting(
                CountError::MissingInput,
            ))),
            &completion_request(r#"{"model": "gpt-4o"}"#),
        );
        assert!(error.to_string().contains("price a response's usage"));
    });
}

#[test]
fn cost_native_error_answers_the_plain_exception_payload() {
    Python::initialize();
    Python::attach(|py| {
        install_fakes(py);
        let error = native_error(
            py,
            payload(
                EngineError::Usage(BillingUsageError::Token {
                    field: "prompt_tokens",
                })
                .to_string(),
            ),
        );
        assert!(error.to_string().contains("\"type\":\"Exception\""));
        assert!(
            error
                .to_string()
                .contains("the prompt_tokens token count is not representable")
        );
    });
}

#[test]
fn cost_conversion_errors_map_a_bad_request_to_a_value_error() {
    Python::initialize();
    Python::attach(|_py| {
        let error = PyErr::from(ConvertError::Json {
            source: json_error(),
        });
        assert!(
            error
                .to_string()
                .starts_with("ValueError: the native request text is not JSON")
        );
    });
}

#[test]
fn cost_conversion_errors_map_a_failed_encode_to_a_runtime_error() {
    Python::initialize();
    Python::attach(|_py| {
        let error = PyErr::from(ConvertError::Encode {
            source: json_error(),
        });
        assert!(
            error
                .to_string()
                .starts_with("RuntimeError: the answer could not be written as JSON")
        );
    });
}

#[test]
fn cost_settings_errors_map_to_a_value_error() {
    Python::initialize();
    Python::attach(|_py| {
        let error = PyErr::from(SettingsError::Encode {
            source: json_error(),
        });
        assert!(
            error
                .to_string()
                .starts_with("ValueError: the model cost map could not be written as JSON")
        );
    });
}
