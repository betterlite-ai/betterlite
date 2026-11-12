#![cfg(test)]

use betterlite_config::sections::prompt_injection::PromptInjectionParams;

use super::hook_support::{JUDGE_MODEL, Scenario, params, threshold, timeout};
use crate::prompt_injection::error::PromptInjectionError;
use crate::prompt_injection::hook::PromptInjectionDetection;
use crate::prompt_injection::pool::HeuristicsPool;

fn detection(params: Option<PromptInjectionParams>) -> PromptInjectionDetection {
    PromptInjectionDetection::new(params, threshold(), HeuristicsPool::new(1, timeout()))
}

fn listed_names() -> [String; 1] {
    [JUDGE_MODEL.to_owned()]
}

#[test]
fn without_params_any_model_list_is_accepted() {
    let mut detection = detection(None);
    detection.update_environment(None).unwrap();
    detection.update_environment(Some(&[])).unwrap();
    detection.update_environment(Some(&listed_names())).unwrap();
}

#[test]
fn a_disabled_llm_api_check_accepts_a_missing_model_list() {
    let mut detection = detection(Some(params(Scenario::AllFalse)));
    detection.update_environment(None).unwrap();
    detection.update_environment(Some(&[])).unwrap();
}

#[test]
fn an_enabled_llm_api_check_requires_the_model_list() {
    let mut detection = detection(Some(params(Scenario::LlmApi)));
    let error = detection
        .update_environment(None)
        .expect_err("a missing model list is refused");
    assert!(matches!(error, PromptInjectionError::ModelListNotSet));
    assert_eq!(
        error.to_string(),
        "PromptInjectionDetection: Model List not set. Required for Prompt Injection detection."
    );
}

#[test]
fn an_enabled_llm_api_check_requires_a_listed_model_name() {
    let mut detection = detection(Some(params(Scenario::LlmApi)));
    let error = detection
        .update_environment(Some(&["other-model".to_owned()]))
        .expect_err("an unlisted model name is refused");
    assert!(matches!(error, PromptInjectionError::InvalidLlmApiName));
    assert_eq!(
        error.to_string(),
        "PromptInjectionDetection: Invalid LLM API Name. LLM API Name must be a 'model_name' in 'model_list'."
    );
}

#[test]
fn an_enabled_llm_api_check_without_a_name_is_refused() {
    let mut detection = detection(Some(params(Scenario::LlmApiWithoutName)));
    let error = detection
        .update_environment(Some(&listed_names()))
        .expect_err("a missing model name is refused");
    assert!(matches!(error, PromptInjectionError::InvalidLlmApiName));
}

#[test]
fn a_listed_model_name_is_accepted() {
    let mut detection = detection(Some(params(Scenario::LlmApi)));
    detection.update_environment(Some(&listed_names())).unwrap();
}
