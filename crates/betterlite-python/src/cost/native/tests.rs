use pyo3::prelude::*;

use super::NativeCost;

const SETTINGS: &str = r#"{"model_cost": {
    "gpt-test": {
        "litellm_provider": "openai",
        "mode": "chat",
        "input_cost_per_token": 0.001,
        "output_cost_per_token": 0.002
    }
}}"#;

const CHAT: &str = r#"{"model": "gpt-test", "custom_llm_provider": "openai",
    "response": {"kind": "chat", "model": "gpt-test",
                 "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}}}"#;

fn cost() -> NativeCost {
    NativeCost::new(SETTINGS).expect("the fixture settings build")
}

#[test]
fn cost_register_adds_the_native_cost_class() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        super::super::register(&module).unwrap();
        assert!(module.hasattr("NativeCost").unwrap());
    });
}

#[test]
fn cost_native_refuses_settings_text_that_is_not_json() {
    Python::initialize();
    Python::attach(|_py| {
        assert!(NativeCost::new("not json").is_err());
    });
}

#[test]
fn cost_native_prices_the_prompt_and_the_completion_apart() {
    Python::initialize();
    Python::attach(|py| {
        let text = cost()
            .cost_per_token(
                py,
                r#"{"model": "gpt-test", "prompt_tokens": 10, "completion_tokens": 5,
                    "custom_llm_provider": "openai"}"#,
            )
            .expect("the call is priced");
        let pair: Vec<f64> = serde_json::from_str(&text).expect("the pair is JSON");
        assert_eq!(pair, vec![0.01, 0.01]);
    });
}

#[test]
fn cost_native_completion_cost_answers_the_charge_and_its_breakdown() {
    Python::initialize();
    Python::attach(|py| {
        let text = cost()
            .completion_cost(py, CHAT)
            .expect("the response is priced");
        let answer: serde_json::Value = serde_json::from_str(&text).expect("the answer is JSON");
        assert_eq!(answer["cost"], serde_json::json!(0.02));
        assert_eq!(answer["breakdown"]["input_cost"], serde_json::json!(0.01));
        assert_eq!(answer["breakdown"]["total_cost"], serde_json::json!(0.02));
        assert_eq!(
            answer["billed_token_rates"]["input_cost_per_token"],
            serde_json::json!(0.001)
        );
    });
}

#[test]
fn cost_native_response_cost_prices_the_same_request() {
    Python::initialize();
    Python::attach(|py| {
        let text = cost()
            .response_cost(py, CHAT)
            .expect("the response is priced");
        let answer: serde_json::Value = serde_json::from_str(&text).expect("the answer is JSON");
        assert_eq!(answer["cost"], serde_json::json!(0.02));
    });
}

#[test]
fn cost_native_usage_object_answers_the_usage_mapping() {
    Python::initialize();
    Python::attach(|py| {
        let text = cost()
            .usage_object(
                py,
                r#"{"kind": "other",
                    "usage": {"prompt_tokens": 4, "completion_tokens": 2, "total_tokens": 6}}"#,
            )
            .expect("the usage reads");
        let answer: serde_json::Value = serde_json::from_str(&text).expect("the answer is JSON");
        assert_eq!(answer["usage"]["total_tokens"], serde_json::json!(6));
    });
}

#[test]
fn cost_native_usage_object_answers_none_without_a_usage() {
    Python::initialize();
    Python::attach(|py| {
        let text = cost()
            .usage_object(py, "{}")
            .expect("the empty request reads");
        let answer: serde_json::Value = serde_json::from_str(&text).expect("the answer is JSON");
        assert!(answer["usage"].is_null());
    });
}

#[test]
fn cost_native_methods_refuse_request_text_that_is_not_json() {
    Python::initialize();
    Python::attach(|py| {
        let native = cost();
        assert!(native.cost_per_token(py, "not json").is_err());
        assert!(native.completion_cost(py, "not json").is_err());
        assert!(native.response_cost(py, "not json").is_err());
        assert!(native.usage_object(py, "not json").is_err());
        let error = native
            .cost_per_token(py, "not json")
            .err()
            .expect("malformed text fails");
        assert!(error.to_string().starts_with("ValueError:"));
    });
}
