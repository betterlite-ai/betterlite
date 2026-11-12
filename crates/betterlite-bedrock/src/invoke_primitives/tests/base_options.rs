use crate::invoke_primitives::config::{
    Params, SUPPORTED_OPENAI_PARAMS, apply_config_to_params, map_openai_params,
};

use super::support::params;

fn value<'params>(parameters: &'params Params, key: &str) -> Option<&'params str> {
    parameters.get(key).map(|raw| raw.get())
}

pub(super) fn base_options() {
    supported_list();
    token_alias_order();
    stream_and_other_names();
    config_defaults();
}

fn supported_list() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        ["max_tokens", "max_completion_tokens", "stream"]
    );
}

fn token_alias_order() {
    let completion_first =
        params(r#"{"max_completion_tokens":9,"max_tokens":5,"temperature":0.2}"#);
    let mapped = map_openai_params(&completion_first, params(r#"{"keep":"x"}"#));
    assert_eq!(value(&mapped, "max_tokens"), Some("5"));
    assert_eq!(value(&mapped, "keep"), Some("\"x\""));
    assert_eq!(value(&mapped, "temperature"), None);
    let max_first = params(r#"{"max_tokens":5,"max_completion_tokens":9}"#);
    assert_eq!(
        value(&map_openai_params(&max_first, params("{}")), "max_tokens"),
        Some("9")
    );
}

fn stream_and_other_names() {
    let requested = params(r#"{"stream":"true","top_p":1,"max_tokens":null}"#);
    let mapped = map_openai_params(&requested, params("{}"));
    assert_eq!(value(&mapped, "stream"), Some("\"true\""));
    assert_eq!(value(&mapped, "max_tokens"), Some("null"));
    assert_eq!(value(&mapped, "top_p"), None);
}

fn config_defaults() {
    let mut inference = params(r#"{"max_tokens":null,"top_k":1}"#);
    apply_config_to_params(
        &params(r#"{"max_tokens":7,"top_k":5,"stream":true}"#),
        &mut inference,
    );
    assert_eq!(value(&inference, "max_tokens"), Some("null"));
    assert_eq!(value(&inference, "top_k"), Some("1"));
    assert_eq!(value(&inference, "stream"), Some("true"));
    let mut missing = params("{}");
    apply_config_to_params(&params(r#"{"max_tokens":7}"#), &mut missing);
    assert_eq!(value(&missing, "max_tokens"), Some("7"));
}
