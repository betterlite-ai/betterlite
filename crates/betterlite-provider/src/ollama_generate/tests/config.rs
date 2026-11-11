use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::support::options;
use crate::ollama_chat::error::OllamaError;
use crate::ollama_generate::config::{
    OllamaGenerateConfig, REQUIRED_PARAMS, SUPPORTED_OPENAI_PARAMS, complete_url, get_error_class,
    map_openai_params,
};

fn full_config() -> OllamaGenerateConfig {
    serde_json::from_str(
        r#"{"mirostat":0,"mirostat_eta":0.1,"mirostat_tau":5.0,"num_ctx":4096,"num_gqa":8,"num_gpu":1,"num_thread":8,"repeat_last_n":64,"repeat_penalty":1.1,"temperature":0.5,"seed":2,"stop":["a","b"],"tfs_z":1.0,"num_predict":42,"top_k":40,"top_p":0.9,"system":"s","template":"t"}"#,
    )
    .unwrap()
}

#[test]
fn get_config_reports_every_set_field_in_definition_order() {
    let found = full_config().get_config();
    let names: Vec<&str> = found.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "mirostat",
            "mirostat_eta",
            "mirostat_tau",
            "num_ctx",
            "num_gqa",
            "num_gpu",
            "num_thread",
            "repeat_last_n",
            "repeat_penalty",
            "temperature",
            "seed",
            "stop",
            "tfs_z",
            "num_predict",
            "top_k",
            "top_p",
            "system",
            "template",
        ]
    );
    assert_eq!(
        found.get("stop"),
        Some(&Json::List(vec![
            Json::Text(String::from("a")),
            Json::Text(String::from("b")),
        ]))
    );
    assert_eq!(
        found.get("num_gpu"),
        Some(&Json::Number(Number::from(1_u64)))
    );
    assert_eq!(
        found.get("mirostat_tau"),
        Some(&Json::Number(Number::from_f64(5.0).unwrap()))
    );
}

#[test]
fn get_config_drops_unset_fields_and_keeps_zero_and_empty() {
    let config: OllamaGenerateConfig =
        serde_json::from_str(r#"{"system":"","num_predict":0}"#).unwrap();
    let found = config.get_config();
    let names: Vec<&str> = found.keys().map(String::as_str).collect();
    assert_eq!(names, ["num_predict", "system"]);
    assert_eq!(found.get("system"), Some(&Json::Text(String::new())));
    assert_eq!(
        found.get("num_predict"),
        Some(&Json::Number(Number::from(0_i32)))
    );
}

#[test]
fn get_config_rejects_an_undeclared_option() {
    let error =
        serde_json::from_str::<OllamaGenerateConfig>(r#"{"num_gpu_layers":3}"#).unwrap_err();
    assert!(error.to_string().contains("num_gpu_layers"));
}

#[test]
fn supported_params_match_litellm_order() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "max_tokens",
            "stream",
            "top_p",
            "temperature",
            "seed",
            "frequency_penalty",
            "stop",
            "response_format",
            "max_completion_tokens",
            "reasoning_effort",
        ]
    );
}

#[test]
fn required_params_are_the_documented_base_url_field() {
    assert_eq!(REQUIRED_PARAMS.len(), 1_usize);
    assert_eq!(REQUIRED_PARAMS[0].field_name, "base_url");
    assert_eq!(REQUIRED_PARAMS[0].field_type, "string");
    assert_eq!(REQUIRED_PARAMS[0].field_description, "Your Ollama API Base");
    assert_eq!(REQUIRED_PARAMS[0].field_value, "http://10.10.11.249:11434");
}

#[rstest]
#[case::max_tokens(r#"{"max_tokens":5}"#, r#"{"num_predict":5}"#)]
#[case::max_completion_tokens(r#"{"max_completion_tokens":7}"#, r#"{"num_predict":7}"#)]
#[case::both_tokens(
    r#"{"max_tokens":5,"max_completion_tokens":7}"#,
    r#"{"num_predict":7}"#
)]
#[case::stream(r#"{"stream":true}"#, r#"{"stream":true}"#)]
#[case::temperature_zero(r#"{"temperature":0}"#, r#"{"temperature":0}"#)]
#[case::seed(r#"{"seed":1}"#, r#"{"seed":1}"#)]
#[case::top_p(r#"{"top_p":0.9}"#, r#"{"top_p":0.9}"#)]
#[case::frequency_penalty_stays(r#"{"frequency_penalty":1.2}"#, r#"{"frequency_penalty":1.2}"#)]
#[case::stop(r#"{"stop":["x"]}"#, r#"{"stop":["x"]}"#)]
#[case::json_object(
    r#"{"response_format":{"type":"json_object"}}"#,
    r#"{"format":"json"}"#
)]
#[case::json_schema(
    r#"{"response_format":{"type":"json_schema","json_schema":{"schema":{"type":"object"}}}}"#,
    r#"{"format":{"type":"object"}}"#
)]
#[case::json_schema_null(
    r#"{"response_format":{"type":"json_schema","json_schema":{"schema":null}}}"#,
    r#"{"format":null}"#
)]
#[case::response_format_text(r#"{"response_format":{"type":"text"}}"#, "{}")]
#[case::response_format_not_an_object(r#"{"response_format":"json"}"#, "{}")]
#[case::reasoning_low(r#"{"reasoning_effort":"low"}"#, r#"{"think":true}"#)]
#[case::reasoning_medium(r#"{"reasoning_effort":"medium"}"#, r#"{"think":true}"#)]
#[case::reasoning_high(r#"{"reasoning_effort":"high"}"#, r#"{"think":true}"#)]
#[case::reasoning_minimal(r#"{"reasoning_effort":"minimal"}"#, r#"{"think":false}"#)]
#[case::reasoning_null(r#"{"reasoning_effort":null}"#, "{}")]
#[case::unknown_param_ignored(r#"{"max_tokens":5,"other":3}"#, r#"{"num_predict":5}"#)]
fn map_openai_params_maps_the_openai_names(#[case] input: &str, #[case] expected: &str) {
    let mut optional = IndexMap::new();
    map_openai_params(&options(input), &mut optional, "llama3").unwrap();
    assert_eq!(Json::Object(optional), Json::Object(options(expected)));
}

#[test]
fn map_openai_params_sends_the_reasoning_effort_text_for_gpt_oss() {
    let mut optional = IndexMap::new();
    map_openai_params(
        &options(r#"{"reasoning_effort":"medium"}"#),
        &mut optional,
        "gpt-oss:20b",
    )
    .unwrap();
    assert_eq!(
        optional.get("think"),
        Some(&Json::Text(String::from("medium")))
    );
}

#[test]
fn map_openai_params_overwrites_an_optional_param() {
    let mut optional = options(r#"{"num_predict":9}"#);
    map_openai_params(&options(r#"{"max_tokens":5}"#), &mut optional, "llama3").unwrap();
    assert_eq!(
        optional.get("num_predict"),
        Some(&Json::Number(Number::from(5_i32)))
    );
}

#[rstest]
#[case::missing_type(r#"{"response_format":{}}"#)]
#[case::missing_json_schema(r#"{"response_format":{"type":"json_schema"}}"#)]
#[case::schema_not_an_object(
    r#"{"response_format":{"type":"json_schema","json_schema":{"schema":"x"}}}"#
)]
#[case::json_schema_not_an_object(r#"{"response_format":{"type":"json_schema","json_schema":[]}}"#)]
#[case::type_not_text(r#"{"response_format":{"type":5}}"#)]
fn map_openai_params_rejects_a_malformed_response_format(#[case] input: &str) {
    let error = map_openai_params(&options(input), &mut IndexMap::new(), "llama3").unwrap_err();
    assert_eq!(error.status(), Some(400));
}

#[test]
fn map_openai_params_rejects_a_non_text_reasoning_effort() {
    let error = map_openai_params(
        &options(r#"{"reasoning_effort":5}"#),
        &mut IndexMap::new(),
        "llama3",
    )
    .unwrap_err();
    assert_eq!(error.status(), Some(400));
}

#[rstest]
#[case::unset(None, "http://localhost:11434/api/generate")]
#[case::plain(Some("http://h:11434"), "http://h:11434/api/generate")]
#[case::trailing_slash(Some("http://h:11434/"), "http://h:11434//api/generate")]
#[case::already_complete(Some("http://h:11434/api/generate"), "http://h:11434/api/generate")]
#[case::suffix_without_slash(Some("http://h/api/generate/"), "http://h/api/generate//api/generate")]
#[case::empty(Some(""), "/api/generate")]
fn complete_url_appends_the_generate_path(#[case] api_base: Option<&str>, #[case] expected: &str) {
    assert_eq!(complete_url(api_base), expected);
}

#[test]
fn get_error_class_carries_the_status_message_and_headers() {
    let incoming = BTreeMap::from([(String::from("x"), String::from("y"))]);
    let OllamaError::Api {
        status_code,
        message,
        headers,
    } = get_error_class("boom", 418, incoming)
    else {
        panic!("expected the API error");
    };
    assert_eq!(status_code, 418);
    assert_eq!(message, "boom");
    assert_eq!(headers.get("x"), Some(&String::from("y")));
}
