#![cfg(test)]

use crate::invoke_primitives::config::Params;
use crate::qwen_deepseek::options::{
    Options, SUPPORTED_OPENAI_PARAMS, map_openai_params, transform_request,
};
use crate::qwen_deepseek::tests::support::{params, prompts};

pub(in crate::qwen_deepseek::tests) fn qwen3_options() {
    class_defaults_are_none();
    supported_names();
    mapping_copies_supported_names();
    request_without_options();
    request_projects_supplied_options();
    explicit_nulls_are_kept();
    request_does_not_mutate_options();
}

fn class_defaults_are_none() {
    let defaults = Options::default();
    assert_eq!(defaults.max_tokens, None);
    assert_eq!(defaults.temperature, None);
    assert_eq!(defaults.top_p, None);
    assert_eq!(defaults.top_k, None);
    assert_eq!(defaults.stop, None);
    let configured = Options {
        max_tokens: Some(64),
        temperature: Some(0.5_f64),
        top_p: Some(0.9_f64),
        top_k: Some(40),
        stop: Some(vec![String::from("END")]),
    };
    assert_eq!(configured.max_tokens, Some(64));
    assert_eq!(configured.temperature, Some(0.5_f64));
    assert_eq!(configured.top_p, Some(0.9_f64));
    assert_eq!(configured.top_k, Some(40));
    assert_eq!(configured.stop, Some(vec![String::from("END")]));
}

fn supported_names() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "max_tokens",
            "temperature",
            "top_p",
            "top_k",
            "stop",
            "stream"
        ]
    );
}

fn mapping_copies_supported_names() {
    let non_default = params(
        r#"{"max_tokens": 64, "temperature": 0.5, "top_p": 0.9, "top_k": 40, "stop": ["END"], "stream": true, "top_logprobs": 2, "aws_access_key_id": "AKIA"}"#,
    );
    let mapped = map_openai_params(&non_default, params(r#"{"seed": "kept"}"#));
    assert_eq!(mapped.len(), 7, "the seed plus the six supported names");
    assert!(mapped.contains_key("seed"));
    assert!(mapped.contains_key("stream"));
    assert!(!mapped.contains_key("top_logprobs"));
    assert!(!mapped.contains_key("aws_access_key_id"));
    assert_eq!(
        mapped.get("max_tokens").map(|value| value.get()),
        Some("64")
    );
    assert_eq!(
        mapped.get("temperature").map(|value| value.get()),
        Some("0.5")
    );
    assert_eq!(mapped.get("top_p").map(|value| value.get()), Some("0.9"));
    assert_eq!(mapped.get("top_k").map(|value| value.get()), Some("40"));
    assert_eq!(
        mapped.get("stop").map(|value| value.get()),
        Some("[\"END\"]")
    );
    assert_eq!(non_default.len(), 8, "the caller's parameters stay as sent");
    assert!(non_default.contains_key("top_logprobs"));
}

fn request_without_options() {
    let body = transform_request(&[], &Params::new()).expect("the empty request converts");
    assert_eq!(body.len(), 1, "only the prompt is sent");
    let prompt: String =
        serde_json::from_str(body.get("prompt").expect("the prompt is present").get())
            .expect("the prompt is a JSON string");
    assert_eq!(prompt, "<|im_start|>assistant\n");
    for name in [
        "max_tokens",
        "max_gen_len",
        "temperature",
        "top_p",
        "top_k",
        "stop",
    ] {
        assert!(!body.contains_key(name), "no class default reaches {name}");
    }
}

fn request_projects_supplied_options() {
    let options = params(
        r#"{"max_tokens": 64, "temperature": 0.5, "top_p": 0.9, "top_k": 40, "stop": ["END"], "stream": true, "aws_access_key_id": "AKIA", "unknown": 1}"#,
    );
    let messages = prompts(r#"[{"role": "user", "content": "hi"}]"#);
    let body = transform_request(&messages, &options).expect("the request converts");
    assert_eq!(body.len(), 6, "the prompt plus the five projected names");
    assert_eq!(body.get("max_gen_len").map(|value| value.get()), Some("64"));
    assert_eq!(
        body.get("temperature").map(|value| value.get()),
        Some("0.5")
    );
    assert_eq!(body.get("top_p").map(|value| value.get()), Some("0.9"));
    assert_eq!(body.get("top_k").map(|value| value.get()), Some("40"));
    assert_eq!(body.get("stop").map(|value| value.get()), Some("[\"END\"]"));
    for name in ["max_tokens", "stream", "aws_access_key_id", "unknown"] {
        assert!(!body.contains_key(name), "{name} is omitted");
    }
    let prompt: String =
        serde_json::from_str(body.get("prompt").expect("the prompt is present").get())
            .expect("the prompt is a JSON string");
    assert_eq!(
        prompt,
        "<|im_start|>user\nhi<|im_end|>\n<|im_start|>assistant\n"
    );
}

fn explicit_nulls_are_kept() {
    let options = params(
        r#"{"max_tokens": null, "temperature": null, "top_p": null, "top_k": null, "stop": null}"#,
    );
    let body = transform_request(&[], &options).expect("the request converts");
    assert_eq!(body.len(), 6, "an explicit null is a supplied value");
    for name in ["max_gen_len", "temperature", "top_p", "top_k", "stop"] {
        assert_eq!(
            body.get(name).map(|value| value.get()),
            Some("null"),
            "{name}"
        );
    }
}

fn request_does_not_mutate_options() {
    let options = params(r#"{"stream": true, "max_tokens": 8}"#);
    let body = transform_request(&[], &options).expect("the request converts");
    assert_eq!(body.len(), 2, "stream is not projected");
    assert_eq!(options.len(), 2, "the options stay as sent");
    assert!(options.contains_key("stream"));
    assert!(options.contains_key("max_tokens"));
}
