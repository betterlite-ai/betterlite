#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::completion::handler::TextCompletionOutcome;
use crate::completion::tests::fixture::{Answer, Fixed, Recorder, ScriptedSource, run};
use crate::endpoints::projection::ClientKind;

mod auth;
mod projection;
mod retries;
mod support;

use support::{
    API_BASE, MODEL, Parts, buffered, completion, contents, data_field, field, nested, number,
    parts_messages, reply_json, stream_items, text, text_messages,
};

#[rstest]
#[case::token_prompt(0)]
#[case::stream_text_choices(1)]
#[case::completion_errors(2)]
#[case::azure_text_prompt_every_call(3)]
#[case::cloudflare_model_null(4)]
#[case::max_retries_default_two(5)]
#[case::invalid_retries_422(6)]
#[case::missing_model_messages_422(7)]
#[case::azure_text_auth(8)]
#[case::azure_text_stream_dispatch(9)]
#[case::azure_text_client_errors(10)]
#[case::azure_text_response_projection(11)]
fn completion_cases(#[case] case: u8) {
    match case {
        0 => token_prompt(),
        1 => stream_text_choices(),
        2 => completion_errors(),
        3 => azure_text_prompt_every_call(),
        4 => cloudflare_model_null(),
        5 => retries::max_retries_default_two(),
        6 => retries::invalid_retries_422(),
        7 => retries::missing_model_messages_422(),
        8 => auth::azure_text_auth(),
        9 => auth::azure_text_stream_dispatch(),
        10 => auth::azure_text_client_errors(),
        _ => projection::azure_text_response_projection(),
    }
}

pub(super) fn recorder() -> Recorder {
    Recorder::default()
}

fn token_prompt() {
    let fixed = Fixed::new();
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, buffered());
    let seen = source.clone();
    let parts = Parts::new();
    let messages = text_messages();
    let mut params = support::object_json(r#"{"temperature":0.5,"max_tokens":16}"#);
    let mut hooks = recorder();
    let outcome = completion(
        source,
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap();
    let TextCompletionOutcome::Buffered(reply) = outcome else {
        panic!("a buffered answer");
    };
    let creates = seen.creates();
    assert_eq!(
        data_field(&creates, "temperature"),
        Json::Number(serde_json::Number::from_f64(0.5).unwrap())
    );
    assert_eq!(text(&data_field(&creates, "prompt")), "Hello");
    assert_eq!(text(&data_field(&creates, "model")), MODEL);
    assert_eq!(number(&data_field(&creates, "max_tokens")), 16);
    assert_eq!(
        nested(&creates[0], &["timeout"]),
        Json::Number(serde_json::Number::from_f64(30.0).unwrap())
    );
    let pre = &hooks.pre[0];
    assert_eq!(text(&nested(pre, &["input"])), "Hello");
    assert_eq!(
        text(&nested(pre, &["additional_args", "headers", "api_key"])),
        "sk-unit"
    );
    assert_eq!(
        nested(pre, &["additional_args", "headers", "azure_ad_token"]),
        Json::Null
    );
    assert_eq!(
        text(&nested(pre, &["additional_args", "api_base"])),
        API_BASE
    );
    assert_eq!(
        text(&nested(pre, &["additional_args", "api_version"])),
        "2024-02-01"
    );
    assert_eq!(
        text(&nested(
            pre,
            &["additional_args", "complete_input_dict", "prompt"]
        )),
        "Hello"
    );
    let post = &hooks.post[0];
    assert_eq!(text(&nested(post, &["input"])), "Hello");
    assert_eq!(text(&nested(post, &["original_response", "id"])), "cmpl-1");
    let reply = reply_json(&reply);
    assert_eq!(
        text(&nested(&reply, &["choices", "0", "message", "content"])),
        "Hello!"
    );
    assert_eq!(
        text(&nested(&reply, &["choices", "0", "finish_reason"])),
        "stop"
    );
    assert_eq!(number(&nested(&reply, &["usage", "total_tokens"])), 5);
    assert_eq!(
        text(&nested(
            &reply,
            &["_hidden_params", "original_response", "object"]
        )),
        "text_completion"
    );

    // the azure_text prompt joins every text part, so a second call proves the factory runs again
    let parts = Parts::new();
    let messages = parts_messages();
    let mut params = support::object_json("{}");
    let mut hooks = recorder();
    let second = completion(
        ScriptedSource::new(ClientKind::AzureOpenAi, buffered()),
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap();
    assert!(matches!(second, TextCompletionOutcome::Buffered(_)));
    assert_eq!(text(&nested(&hooks.pre[0], &["input"])), "Sys Hi");
}

fn stream_text_choices() {
    let fixed = Fixed::new();
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, Answer::Stream(stream_items()));
    let parts = Parts::new();
    let messages = text_messages();
    let mut params = support::object_json(r#"{"stream":true}"#);
    let mut hooks = recorder();
    let outcome = completion(
        source,
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap();
    let TextCompletionOutcome::Stream(stream) = outcome else {
        panic!("a stream");
    };
    let chunks = run(support::chunks(stream));
    assert_eq!(contents(&chunks), ["Hel", "lo"]);
    assert_eq!(chunks[0].model, MODEL);
    assert_eq!(
        chunks[0].hidden_params.get("custom_llm_provider"),
        Some(
            &betterlite_streaming::stream_chunk_shaping::records::JsonValue::String(
                "azure_text".to_owned()
            )
        )
    );
    let last = chunks.last().unwrap();
    assert!(
        last.choices[0]
            .finish_reason
            .as_deref()
            .is_some_and(|reason| !reason.is_empty())
    );
    let pre = &hooks.pre[0];
    assert_eq!(text(&nested(pre, &["input"])), "Hello");
    assert_eq!(
        nested(pre, &["additional_args", "acompletion"]),
        Json::Bool(true)
    );
    assert_eq!(
        text(&nested(
            pre,
            &["additional_args", "headers", "Authorization"]
        )),
        "Bearer sk-unit"
    );
    assert_eq!(
        text(&nested(
            pre,
            &["additional_args", "complete_input_dict", "stream"]
        )),
        "true"
    );
    assert!(hooks.post.is_empty());
}

fn completion_errors() {
    let fixed = Fixed::new();
    let mut hooks = recorder();
    let parts = Parts::new();
    let messages = text_messages();
    let mut params = support::object_json("{}");
    let status = Answer::Status {
        status: 429,
        message: "slow down".to_owned(),
        headers: vec![("retry-after".to_owned(), "7".to_owned())],
    };
    let error = completion(
        ScriptedSource::new(ClientKind::AzureOpenAi, status),
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap_err();
    assert_eq!(error.status, Some(429));
    assert_eq!(error.message, "slow down");
    assert_eq!(
        error
            .headers
            .as_ref()
            .and_then(|headers| headers.0.get("retry-after"))
            .map(String::as_str),
        Some("7")
    );
    assert_eq!(hooks.pre.len(), 1);
    assert!(hooks.post.is_empty());

    let mut params = support::object_json("{}");
    let error = completion(
        ScriptedSource::new(ClientKind::AzureOpenAi, Answer::Other("boom".to_owned())),
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap_err();
    assert_eq!(error.status, Some(500));
    assert_eq!(error.message, "boom");
}

fn azure_text_prompt_every_call() {
    let fixed = Fixed::new();
    let parts = Parts::new();
    let messages = text_messages();
    // both buffered paths resolve the azure_text prompt, and the params overlay the built data
    let mut params = support::object_json(r#"{"prompt":"overridden"}"#);
    let mut hooks = recorder();
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, buffered());
    let seen = source.clone();
    completion(
        source,
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap();
    let creates = seen.creates();
    assert_eq!(text(&data_field(&creates, "prompt")), "overridden");
    // the sync pre call logs the prompt it built, not the overlaid request field
    assert_eq!(text(&nested(&hooks.pre[0], &["input"])), "Hello");
    assert_eq!(
        text(&nested(
            &hooks.pre[0],
            &["additional_args", "complete_input_dict", "prompt"]
        )),
        "overridden"
    );

    let mut params = support::object_json(r#"{"prompt":"overridden"}"#);
    let mut hooks = recorder();
    let source = ScriptedSource::new(ClientKind::AsyncAzureOpenAi, buffered());
    let seen = source.clone();
    let outcome = completion(
        source,
        parts.call(Some(&messages), &mut params, true),
        &fixed,
        &mut hooks,
    )
    .unwrap();
    assert!(matches!(outcome, TextCompletionOutcome::Buffered(_)));
    assert_eq!(data_field(&seen.creates(), "max_tokens"), Json::Null);
    // the async pre call logs data["prompt"], so it carries the overlay
    assert_eq!(text(&nested(&hooks.pre[0], &["input"])), "overridden");
    assert!(hooks.post.is_empty());
}

fn cloudflare_model_null() {
    let fixed = Fixed::new();
    let mut parts = Parts::new();
    parts.api_base = Some("https://gateway.ai.cloudflare.com/v1/account/gw/resource".to_owned());
    let messages = text_messages();
    let mut params = support::object_json("{}");
    let mut hooks = recorder();
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, buffered());
    let seen = source.clone();
    completion(
        source,
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap();
    let gateways = seen.gateways();
    assert_eq!(gateways.len(), 1);
    assert_eq!(text(&field(&gateways[0], "model")), MODEL);
    assert_eq!(
        text(&field(&gateways[0], "api_base")),
        "https://gateway.ai.cloudflare.com/v1/account/gw/resource"
    );
    assert_eq!(number(&field(&gateways[0], "max_retries")), 2);
    assert!(
        seen.gets().is_empty(),
        "the gateway client is used as it is"
    );
    let creates = seen.creates();
    assert_eq!(data_field(&creates, "model"), Json::Null);
    assert_eq!(text(&data_field(&creates, "prompt")), "Hello");
}
