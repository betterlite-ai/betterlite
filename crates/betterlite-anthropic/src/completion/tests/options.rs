#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::anthropic_completion::OptionValues;
use indexmap::IndexMap;

use crate::completion::options::{OptionDefaults, SUPPORTED_OPENAI_PARAMS, map_openai_params};
use crate::completion::prompt::PromptSource;
use crate::completion::request::transform_request;

use super::{USER_PROMPT, clock, factory, object, user_messages};

pub(super) fn mapped_params() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "stream",
            "max_tokens",
            "max_completion_tokens",
            "stop",
            "temperature",
            "top_p",
            "extra_headers",
            "user"
        ]
    );
    let params = object(
        r#"{"max_tokens":10,"stream":true,"max_completion_tokens":20,"stop":"END","temperature":0.5,"top_p":0.25,"user":null,"extra_headers":{"x":"y"},"n":2}"#,
    );
    let mapped = map_openai_params(&params, IndexMap::new(), false).unwrap();
    let keys = mapped.keys().map(String::as_str).collect::<Vec<_>>();
    let order = [
        "max_tokens_to_sample",
        "stream",
        "stop_sequences",
        "temperature",
        "top_p",
        "metadata",
    ];
    assert_eq!(keys, order);
    let expected = r#"{"max_tokens_to_sample":20,"stream":true,"stop_sequences":["END"],"temperature":0.5,"top_p":0.25,"metadata":{"user_id":null}}"#;
    assert_eq!(mapped, object(expected));
    let streamless = object(r#"{"stream":false}"#);
    let kept = map_openai_params(&streamless, object(r#"{"top_k":5}"#), false);
    assert_eq!(kept.unwrap(), object(r#"{"top_k":5}"#));
}

pub(super) fn stop_params() {
    let blank = object(r#"{"stop":" "}"#);
    let dropped = map_openai_params(&blank, IndexMap::new(), true).unwrap();
    assert!(dropped.is_empty());
    let retained = map_openai_params(&blank, IndexMap::new(), false).unwrap();
    assert_eq!(retained, object(r#"{"stop_sequences":[" "]}"#));
    let listed = object(r#"{"stop":["a","b"]}"#);
    let stops = map_openai_params(&listed, IndexMap::new(), false).unwrap();
    assert_eq!(stops, object(r#"{"stop_sequences":["a","b"]}"#));
    let invalid = object(r#"{"stop":["a",1]}"#);
    assert!(map_openai_params(&invalid, IndexMap::new(), false).is_err());
}

pub(super) async fn request_body() {
    let prompts = factory();
    let none = BTreeMap::new();
    let source = PromptSource {
        custom_prompt_dict: &none,
        prompts: &prompts,
        now: clock(),
    };
    let unset = OptionDefaults::resolve(&OptionValues::default(), &OptionValues::default(), 4096);
    let messages = user_messages();
    let body = transform_request(&source, "claude-2", &messages, IndexMap::new(), &unset).await;
    let body = body.unwrap();
    let keys = body.keys().map(String::as_str).collect::<Vec<_>>();
    assert_eq!(keys, ["model", "prompt", "max_tokens_to_sample"]);
    let prompt = serde_json::to_string(USER_PROMPT).unwrap();
    let sent = format!(r#"{{"model":"claude-2","prompt":{prompt},"max_tokens_to_sample":4096}}"#);
    assert_eq!(body, object(&sent));
}

pub(super) async fn request_defaults() {
    let prompts = factory();
    let none = BTreeMap::new();
    let source = PromptSource {
        custom_prompt_dict: &none,
        prompts: &prompts,
        now: clock(),
    };
    let deployment = OptionValues {
        max_tokens_to_sample: Some(256),
        temperature: Some(0.25),
        ..OptionValues::default()
    };
    let global = OptionValues {
        max_tokens_to_sample: Some(512),
        temperature: Some(0.75),
        top_p: Some(0.5),
        top_k: Some(40),
        stop_sequences: Some(vec!["STOP".to_owned()]),
        metadata: Some(object(r#"{"user_id":"configured"}"#)),
    };
    let defaults = OptionDefaults::resolve(&deployment, &global, 4096);
    let own = object(r#"{"top_k":7,"model":"override"}"#);
    let messages = user_messages();
    let body = transform_request(&source, "claude-2", &messages, own, &defaults).await;
    let body = body.unwrap();
    let prompt = serde_json::to_string(USER_PROMPT).unwrap();
    let sent = format!(
        r#"{{"model":"override","prompt":{prompt},"top_k":7,"max_tokens_to_sample":256,"stop_sequences":["STOP"],"temperature":0.25,"top_p":0.5,"metadata":{{"user_id":"configured"}}}}"#
    );
    assert_eq!(body, object(&sent));
    let keys = body.keys().map(String::as_str).collect::<Vec<_>>();
    assert_eq!(&keys[..3], ["model", "prompt", "top_k"]);
}
