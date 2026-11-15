#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use chrono::{DateTime, Utc};
use rstest::rstest;
use serde_json::Number;

use super::config::{
    MAX_NEW_TOKENS_ENV, PetalsConfig, SUPPORTED_OPENAI_PARAMS, map_openai_params,
    max_new_tokens_default,
};
use super::error::{PetalsSetupError, get_error_class};
use super::request::{CustomPrompt, RequestContext};

mod reply;

fn params(wire: &str) -> Object {
    serde_json::from_str(wire).unwrap()
}

fn environment(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn config(max_new_tokens: i64) -> PetalsConfig {
    PetalsConfig {
        max_length: None,
        max_new_tokens,
        do_sample: None,
        temperature: None,
        top_k: None,
        top_p: None,
        repetition_penalty: None,
    }
}

fn messages(wire: &str) -> Vec<Message> {
    serde_json::from_str(wire).unwrap()
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

fn factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(builtin_entries()),
        hub: Hub::Unreachable,
    }
}

fn context<'request>(
    config: &'request PetalsConfig,
    prompts: &'request PromptFactory,
    custom: &'request BTreeMap<String, CustomPrompt>,
) -> RequestContext<'request> {
    RequestContext {
        config,
        prompts,
        custom_prompts: custom,
        now: now(),
    }
}

fn custom_prompt_dict(wire: &str) -> BTreeMap<String, CustomPrompt> {
    serde_json::from_str(wire).unwrap()
}

fn full_config() -> PetalsConfig {
    PetalsConfig {
        max_length: Some(Number::from(10)),
        max_new_tokens: 7,
        do_sample: Some(true),
        temperature: Some(Number::from_f64(0.1).unwrap()),
        top_k: Some(Number::from(5)),
        top_p: Some(Number::from_f64(0.9).unwrap()),
        repetition_penalty: Some(Number::from_f64(1.2).unwrap()),
    }
}

#[test]
fn supported_openai_params_are_litellm_order() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        ["max_tokens", "temperature", "top_p", "stream"]
    );
}

#[rstest]
#[case::unset(&[], 4096)]
#[case::configured(&[(MAX_NEW_TOKENS_ENV, "77")], 77)]
#[case::zero(&[(MAX_NEW_TOKENS_ENV, "0")], 0)]
#[case::blank(&[(MAX_NEW_TOKENS_ENV, "")], 4096)]
#[case::whitespace(&[(MAX_NEW_TOKENS_ENV, " ")], 4096)]
fn max_new_tokens_default_reads_the_environment(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: i64,
) {
    let environment = environment(pairs);
    assert_eq!(max_new_tokens_default(&environment).unwrap(), expected);
}

#[test]
fn max_new_tokens_default_rejects_a_value_that_is_not_a_number() {
    let environment = environment(&[(MAX_NEW_TOKENS_ENV, "many")]);
    let error = max_new_tokens_default(&environment).unwrap_err();
    assert!(error.to_string().contains(MAX_NEW_TOKENS_ENV), "{error}");
}

#[test]
fn get_config_lists_the_callers_max_new_tokens() {
    assert_eq!(config(77).get_config(), params(r#"{"max_new_tokens":77}"#));
}

#[test]
fn get_config_lists_every_set_value() {
    assert_eq!(
        full_config().get_config(),
        params(
            r#"{"max_length":10,"max_new_tokens":7,"do_sample":true,"temperature":0.1,"top_k":5,"top_p":0.9,"repetition_penalty":1.2}"#
        )
    );
}

#[test]
fn get_config_keeps_zero_and_false() {
    let config = PetalsConfig {
        max_length: Some(Number::from(0)),
        max_new_tokens: 0,
        do_sample: Some(false),
        temperature: Some(Number::from(0)),
        top_k: Some(Number::from(0)),
        top_p: Some(Number::from(0)),
        repetition_penalty: Some(Number::from(0)),
    };
    assert_eq!(
        config.get_config(),
        params(
            r#"{"max_length":0,"max_new_tokens":0,"do_sample":false,"temperature":0,"top_k":0,"top_p":0,"repetition_penalty":0}"#
        )
    );
}

#[rstest]
#[case::max_tokens(r#"{"max_tokens":5}"#, r#"{"max_new_tokens":5}"#)]
#[case::max_tokens_zero(r#"{"max_tokens":0}"#, r#"{"max_new_tokens":0}"#)]
#[case::temperature(r#"{"temperature":0.2}"#, r#"{"temperature":0.2}"#)]
#[case::top_p(r#"{"top_p":0.9}"#, r#"{"top_p":0.9}"#)]
#[case::stream_true(r#"{"stream":true}"#, r#"{"stream":true}"#)]
#[case::stream_false(r#"{"stream":false}"#, r#"{"stream":false}"#)]
#[case::max_completion_tokens_is_unmapped(r#"{"max_completion_tokens":7}"#, "{}")]
#[case::top_k_is_unmapped(r#"{"top_k":5}"#, "{}")]
#[case::empty("{}", "{}")]
fn openai_params_map_into_the_options(#[case] wire: &str, #[case] expected: &str) {
    let mut options = Object::new();
    map_openai_params(&params(wire), &mut options);
    assert_eq!(options, params(expected));
}

#[test]
fn openai_params_keep_the_existing_options() {
    let mut options = params(r#"{"temperature":0.5}"#);
    map_openai_params(&params(r#"{"top_p":0.9}"#), &mut options);
    assert_eq!(options, params(r#"{"temperature":0.5,"top_p":0.9}"#));
}

#[tokio::test]
async fn request_body_carries_the_config_defaults() {
    let config = config(4096);
    let prompts = factory();
    let custom = BTreeMap::new();
    let context = context(&config, &prompts, &custom);
    let body = context
        .transform_request(
            "meta-llama/llama-2-7b-chat",
            &messages(r#"[{"role":"user","content":"hi"}]"#),
            Some("http://petals.test/v1/generate"),
            &params("{}"),
        )
        .await
        .unwrap();
    assert_eq!(body.model, "meta-llama/llama-2-7b-chat");
    assert_eq!(body.inputs.as_deref(), Some("<s>[INST] hi [/INST]\n"));
    assert_eq!(body.parameters, params(r#"{"max_new_tokens":4096}"#));
}

#[tokio::test]
async fn request_body_keeps_the_caller_options_over_the_config() {
    let config = full_config();
    let prompts = factory();
    let custom = BTreeMap::new();
    let context = context(&config, &prompts, &custom);
    let mut options = Object::new();
    map_openai_params(
        &params(r#"{"max_tokens":5,"temperature":0.5}"#),
        &mut options,
    );
    let body = context
        .transform_request(
            "meta-llama/llama-2-7b-chat",
            &messages(r#"[{"role":"user","content":"hi"}]"#),
            Some("http://petals.test/v1/generate"),
            &options,
        )
        .await
        .unwrap();
    assert_eq!(
        body.parameters,
        params(
            r#"{"max_new_tokens":5,"temperature":0.5,"max_length":10,"do_sample":true,"top_k":5,"top_p":0.9,"repetition_penalty":1.2}"#
        )
    );
}

#[tokio::test]
async fn request_body_pops_stream_but_keeps_the_mapped_options() {
    let config = config(4096);
    let prompts = factory();
    let custom = BTreeMap::new();
    let context = context(&config, &prompts, &custom);
    let mut options = Object::new();
    map_openai_params(
        &params(r#"{"stream":true,"temperature":0.2}"#),
        &mut options,
    );
    let body = context
        .transform_request(
            "meta-llama/llama-2-7b-chat",
            &messages(r#"[{"role":"user","content":"hi"}]"#),
            Some("http://petals.test/v1/generate"),
            &options,
        )
        .await
        .unwrap();
    assert_eq!(
        body.parameters,
        params(r#"{"temperature":0.2,"max_new_tokens":4096}"#)
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[tokio::test]
async fn request_body_without_a_base_is_the_engine_error(#[case] api_base: Option<&str>) {
    let config = config(4096);
    let prompts = factory();
    let custom = BTreeMap::new();
    let context = context(&config, &prompts, &custom);
    let error = context
        .transform_request(
            "meta-llama/llama-2-7b-chat",
            &messages(r#"[{"role":"user","content":"hi"}]"#),
            api_base,
            &params("{}"),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PetalsSetupError::Engine {
            status_code: 400,
            ..
        }
    ));
    assert_eq!(error.status(), Some(400));
    assert_eq!(
        error.to_string(),
        "Importing torch, transformers, petals failed\nTry pip installing petals \npip install git+https://github.com/bigscience-workshop/petals"
    );
}

#[tokio::test]
async fn a_custom_prompt_dictionary_entry_builds_the_prompt() {
    let config = config(4096);
    let prompts = factory();
    let custom = custom_prompt_dict(
        r#"{"m":{"roles":{"user":{"pre_message":"U:","post_message":"!"}},"initial_prompt_value":"I","final_prompt_value":"F"}}"#,
    );
    let context = context(&config, &prompts, &custom);
    let body = context
        .transform_request(
            "m",
            &messages(r#"[{"role":"user","content":"hi"}]"#),
            Some("http://petals.test/v1/generate"),
            &params("{}"),
        )
        .await
        .unwrap();
    assert_eq!(body.inputs.as_deref(), Some("IU:hi!F"));
}

#[tokio::test]
async fn a_custom_prompt_for_another_model_uses_the_prompt_factory() {
    let config = config(4096);
    let prompts = factory();
    let custom = custom_prompt_dict(
        r#"{"other":{"roles":{},"initial_prompt_value":"I","final_prompt_value":"F"}}"#,
    );
    let context = context(&config, &prompts, &custom);
    let body = context
        .transform_request(
            "meta-llama/llama-2-7b-chat",
            &messages(r#"[{"role":"user","content":"hi"}]"#),
            Some("http://petals.test/v1/generate"),
            &params("{}"),
        )
        .await
        .unwrap();
    assert_eq!(body.inputs.as_deref(), Some("<s>[INST] hi [/INST]\n"));
}

#[rstest]
#[case::missing_roles(
    r#"{"m":{"initial_prompt_value":"I","final_prompt_value":"F"}}"#,
    "roles"
)]
#[case::missing_initial(
    r#"{"m":{"roles":{},"final_prompt_value":"F"}}"#,
    "initial_prompt_value"
)]
#[case::missing_final(
    r#"{"m":{"roles":{},"initial_prompt_value":"I"}}"#,
    "final_prompt_value"
)]
#[tokio::test]
async fn a_custom_prompt_entry_missing_a_field_fails(#[case] wire: &str, #[case] field: &str) {
    let config = config(4096);
    let prompts = factory();
    let custom = custom_prompt_dict(wire);
    let context = context(&config, &prompts, &custom);
    let error = context
        .transform_request(
            "m",
            &messages(r#"[{"role":"user","content":"hi"}]"#),
            Some("http://petals.test/v1/generate"),
            &params("{}"),
        )
        .await
        .unwrap_err();
    assert_eq!(error.status(), None);
    assert!(matches!(
        error,
        PetalsSetupError::CustomPromptField { model, field: found } if model == "m" && found == field
    ));
}

#[tokio::test]
async fn a_prompt_error_wins_over_the_engine_error() {
    let config = config(4096);
    let prompts = factory();
    let custom = custom_prompt_dict(r#"{"m":{"roles":{}}}"#);
    let context = context(&config, &prompts, &custom);
    let error = context
        .transform_request(
            "m",
            &messages(r#"[{"role":"user","content":"hi"}]"#),
            None,
            &params("{}"),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, PetalsSetupError::CustomPromptField { .. }));
}

#[tokio::test]
async fn request_body_without_a_prompt_factory_match_has_a_null_input() {
    let config = config(4096);
    let prompts = factory();
    let custom = BTreeMap::new();
    let context = context(&config, &prompts, &custom);
    let body = context
        .transform_request(
            "tiiuae/falcon-7b",
            &messages(r#"[{"role":"user","content":"hi"}]"#),
            Some("http://petals.test/v1/generate"),
            &params("{}"),
        )
        .await
        .unwrap();
    assert_eq!(body.inputs, None);
}

#[test]
fn the_error_class_carries_the_status_message_and_headers() {
    let headers = BTreeMap::from([(String::from("x"), String::from("1"))]);
    let error = get_error_class("boom", 418, &headers);
    assert_eq!(error.status_code, 418);
    assert_eq!(error.message, "boom");
    assert_eq!(error.headers, headers);
}
