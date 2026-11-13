#![cfg(test)]

use betterlite_prompts::arguments::json::Object;
use rstest::rstest;

use super::{SUPPORTED_OPENAI_PARAMS, map_openai_params};

fn params(wire: &str) -> Object {
    serde_json::from_str(wire).unwrap()
}

#[test]
fn supported_params_match_litellm() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "stream",
            "temperature",
            "max_tokens",
            "top_p",
            "stop",
            "seed",
            "tools",
            "tool_choice",
            "functions",
            "function_call"
        ]
    );
}

#[rstest]
#[case::stream(r#"{"stream":true}"#, r#"{"stream":true}"#)]
#[case::temperature(r#"{"temperature":0.2}"#, r#"{"temperature":0.2}"#)]
#[case::top_p(r#"{"top_p":0.9}"#, r#"{"top_p":0.9}"#)]
#[case::stop_list(r#"{"stop":["x","y"]}"#, r#"{"stop_sequences":["x","y"]}"#)]
#[case::stop_string(r#"{"stop":"x"}"#, r#"{"stop_sequences":"x"}"#)]
#[case::seed_is_dropped(r#"{"seed":42}"#, "{}")]
#[case::tools_are_dropped(r#"{"tools":[{"type":"function"}]}"#, "{}")]
#[case::tool_choice_is_dropped(r#"{"tool_choice":"auto"}"#, "{}")]
#[case::functions_are_dropped(r#"{"functions":[{"name":"f"}]}"#, "{}")]
#[case::function_call_is_dropped(r#"{"function_call":"auto"}"#, "{}")]
fn supported_params_map_to_the_replicate_names(#[case] wire: &str, #[case] expected: &str) {
    let mut options = Object::new();
    map_openai_params(&params(wire), &mut options, "owner/model");
    assert_eq!(options, params(expected));
}

#[rstest]
#[case::plain("owner/model", "max_new_tokens")]
#[case::vicuna("owner/vicuna-13b", "max_length")]
#[case::flan("owner/flan-t5-xl", "max_length")]
#[case::codellama("meta/codellama-13b-instruct", "max_tokens")]
#[case::vicuna_wins_over_codellama("vicuna-meta/codellama-13b", "max_length")]
#[case::flan_wins_over_codellama("flan-meta/codellama-13b", "max_length")]
fn max_tokens_goes_to_the_model_family_key(#[case] model: &str, #[case] target: &str) {
    let mut options = Object::new();
    map_openai_params(&params(r#"{"max_tokens":9}"#), &mut options, model);
    assert_eq!(options, params(&format!(r#"{{"{target}":9}}"#)));
}

#[test]
fn an_existing_option_key_is_left_as_the_call_set_it() {
    let mut options = params(r#"{"max_tokens":3}"#);
    map_openai_params(&params(r#"{"max_tokens":9}"#), &mut options, "owner/model");
    assert_eq!(options, params(r#"{"max_tokens":3,"max_new_tokens":9}"#));
}

#[test]
fn every_mapped_parameter_lands_in_source_order() {
    let mut options = Object::new();
    map_openai_params(
        &params(r#"{"stream":true,"max_tokens":2,"temperature":0.1,"top_p":0.2,"stop":["s"]}"#),
        &mut options,
        "owner/model",
    );
    assert_eq!(
        options,
        params(
            r#"{"stream":true,"max_new_tokens":2,"temperature":0.1,"top_p":0.2,"stop_sequences":["s"]}"#
        )
    );
    assert_eq!(
        options.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "stream",
            "max_new_tokens",
            "temperature",
            "top_p",
            "stop_sequences"
        ]
    );
}
