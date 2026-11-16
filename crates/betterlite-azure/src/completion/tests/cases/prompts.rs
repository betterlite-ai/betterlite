use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::text_completion_transport::logging::SecretHeaders;

use super::support::{
    API_BASE, MODEL, Run, async_buffered, field, float, nested, number, parts_messages,
    sync_buffered, text,
};

fn token_prompt_logs(run: &Run) {
    assert_eq!(text(&run.pre(&["input"])), "Hello");
    assert_eq!(
        text(&run.pre(&["additional_args", "headers", "api_key"])),
        "sk-unit"
    );
    assert_eq!(
        run.pre(&["additional_args", "headers", "azure_ad_token"]),
        Json::Null
    );
    assert_eq!(text(&run.pre(&["additional_args", "api_base"])), API_BASE);
    assert_eq!(
        text(&run.pre(&["additional_args", "api_version"])),
        "2024-02-01"
    );
    let sent = ["additional_args", "complete_input_dict", "prompt"];
    assert_eq!(text(&run.pre(&sent)), "Hello");
    let post = &run.hooks.post[0];
    assert_eq!(text(&nested(post, &["input"])), "Hello");
    assert_eq!(text(&nested(post, &["original_response", "id"])), "cmpl-1");
}

fn token_prompt_reply(run: &Run) {
    let reply = run.reply_json();
    let message = nested(&reply, &["choices", "0", "message", "content"]);
    assert_eq!(text(&message), "Hello!");
    let finish = nested(&reply, &["choices", "0", "finish_reason"]);
    assert_eq!(text(&finish), "stop");
    assert_eq!(number(&nested(&reply, &["usage", "total_tokens"])), 5);
    let original = nested(&reply, &["_hidden_params", "original_response", "object"]);
    assert_eq!(text(&original), "text_completion");
}

fn logged_headers() {
    let mut scenario = sync_buffered("{}");
    let trace = BTreeMap::from([("x-trace".to_owned(), "abc".to_owned())]);
    scenario.parts.headers = Some(SecretHeaders(trace));
    let run = scenario.execute();
    let logged = nested(
        &run.hooks.post[0],
        &["additional_args", "headers", "x-trace"],
    );
    assert_eq!(text(&logged), "abc");
}

pub(super) fn token_prompt() {
    let run = sync_buffered(r#"{"temperature":0.5,"max_tokens":16}"#).execute();
    assert_eq!(run.create("temperature"), float(0.5));
    assert_eq!(text(&run.create("prompt")), "Hello");
    assert_eq!(text(&run.create("model")), MODEL);
    assert_eq!(number(&run.create("max_tokens")), 16);
    assert_eq!(nested(&run.source.creates()[0], &["timeout"]), float(30.0));
    token_prompt_logs(&run);
    token_prompt_reply(&run);
    logged_headers();
    let mut joined = sync_buffered("{}");
    joined.messages = Some(parts_messages());
    assert_eq!(text(&joined.execute().pre(&["input"])), "Sys Hi");
}

pub(super) fn azure_text_prompt_every_call() {
    let sync_run = sync_buffered(r#"{"prompt":"overridden"}"#).execute();
    assert_eq!(text(&sync_run.create("prompt")), "overridden");
    assert_eq!(text(&sync_run.pre(&["input"])), "Hello");
    let sent = ["additional_args", "complete_input_dict", "prompt"];
    assert_eq!(text(&sync_run.pre(&sent)), "overridden");
    let async_run = async_buffered(r#"{"prompt":"overridden"}"#).execute();
    assert!(async_run.reply.is_some());
    assert_eq!(async_run.create("max_tokens"), Json::Null);
    assert_eq!(text(&async_run.pre(&["input"])), "overridden");
    assert!(async_run.hooks.post.is_empty());
}

pub(super) fn cloudflare_model_null() {
    let gateway = "https://gateway.ai.cloudflare.com/v1/account/gw/resource";
    let mut scenario = sync_buffered("{}");
    scenario.parts.api_base = Some(gateway.to_owned());
    let run = scenario.execute();
    let gateways = run.source.gateways();
    assert_eq!(gateways.len(), 1);
    let record = &gateways[0];
    assert_eq!(text(&field(record, "model")), MODEL);
    assert_eq!(text(&field(record, "api_base")), gateway);
    assert_eq!(number(&field(record, "max_retries")), 2);
    assert!(run.source.gets().is_empty());
    assert_eq!(run.create("model"), Json::Null);
    assert_eq!(text(&run.create("prompt")), "Hello");
}
