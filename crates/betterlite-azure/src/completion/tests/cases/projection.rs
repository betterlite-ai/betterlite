#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::completion::handler::TextCompletionOutcome;
use crate::completion::response::ChatReply;
use crate::completion::tests::fixture::{Answer, CLOCK, Fixed, ScriptedSource, object};
use crate::endpoints::projection::ClientKind;

use super::support::{
    MODEL, Parts, completion, field, nested, number, reply_json, text, text_messages,
};

const TWO_CHOICES: &str = r#"{"id":"cmpl-9","object":"text_completion","created":1234,"model":"gpt-35-turbo-instruct","system_fingerprint":"fp_1","choices":[{"text":"A","index":5,"logprobs":null,"finish_reason":"length"},{"text":"B","index":9,"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;

const NO_USAGE: &str = r#"{"id":"cmpl-2","object":"text_completion","created":777,"model":"gpt-35-turbo-instruct","choices":[{"text":"x","index":0,"finish_reason":"content_filter"}],"usage":null}"#;

const EXTRA_USAGE: &str = r#"{"id":"cmpl-3","object":"text_completion","created":888,"model":"gpt-35-turbo-instruct","choices":[{"text":"y","index":0,"finish_reason":"stop","logprobs":null}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5,"completion_tokens_details":null,"prompt_tokens_details":null,"reasoning_tokens":2,"queue_time":0.25}}"#;

fn buffered_reply(kind: ClientKind, body: &str, acompletion: bool, fixed: &Fixed) -> ChatReply {
    let parts = Parts::new();
    let messages = text_messages();
    let mut params = object("{}");
    let mut hooks = super::recorder();
    let outcome = completion(
        ScriptedSource::new(kind, Answer::Buffered(body.to_owned())),
        parts.call(Some(&messages), &mut params, acompletion),
        fixed,
        &mut hooks,
    )
    .unwrap();
    let TextCompletionOutcome::Buffered(reply) = outcome else {
        panic!("a buffered reply");
    };
    reply
}

pub(super) fn azure_text_response_projection() {
    let fixed = Fixed::new();
    let reply = buffered_reply(ClientKind::AzureOpenAi, TWO_CHOICES, false, &fixed);
    let projected = reply_json(&reply);

    // the target the conversion mutates keeps its own object, created and system_fingerprint
    assert_eq!(text(&field(&projected, "id")), "cmpl-9");
    assert_eq!(text(&field(&projected, "object")), "chat.completion");
    assert_eq!(number(&field(&projected, "created")), CLOCK);
    assert_eq!(text(&field(&projected, "model")), MODEL);
    assert_eq!(field(&projected, "system_fingerprint"), Json::Null);

    // every choice becomes an assistant message with a sequential index
    let choices = field(&projected, "choices");
    let Json::List(items) = &choices else {
        panic!("a list of choices");
    };
    assert_eq!(items.len(), 2);
    assert_eq!(
        text(&nested(&projected, &["choices", "0", "message", "content"])),
        "A"
    );
    assert_eq!(
        text(&nested(&projected, &["choices", "0", "message", "role"])),
        "assistant"
    );
    assert_eq!(
        nested(&projected, &["choices", "0", "message", "tool_calls"]),
        Json::Null
    );
    assert_eq!(
        nested(&projected, &["choices", "0", "message", "function_call"]),
        Json::Null
    );
    assert_eq!(
        text(&nested(&projected, &["choices", "0", "finish_reason"])),
        "length"
    );
    assert_eq!(number(&nested(&projected, &["choices", "0", "index"])), 0);
    let Json::Object(first) = nested(&projected, &["choices", "0"]) else {
        panic!("a choice object");
    };
    assert!(
        !first.contains_key("logprobs"),
        "a null logprobs is dropped"
    );
    assert_eq!(
        text(&nested(&projected, &["choices", "1", "message", "content"])),
        "B"
    );
    assert_eq!(
        text(&nested(&projected, &["choices", "1", "finish_reason"])),
        "stop"
    );
    assert_eq!(number(&nested(&projected, &["choices", "1", "index"])), 1);

    // the sync conversion zero-fills the usage and keeps the details keys as null
    assert_eq!(number(&nested(&projected, &["usage", "prompt_tokens"])), 3);
    assert_eq!(number(&nested(&projected, &["usage", "total_tokens"])), 5);
    assert_eq!(
        nested(&projected, &["usage", "completion_tokens_details"]),
        Json::Null
    );
    assert_eq!(
        nested(&projected, &["usage", "prompt_tokens_details"]),
        Json::Null
    );

    // the sync hidden original is the normalized TextCompletionResponse the conversion stored
    assert_eq!(
        text(&nested(
            &projected,
            &["_hidden_params", "original_response", "object"]
        )),
        "text_completion"
    );
    assert_eq!(
        number(&nested(
            &projected,
            &["_hidden_params", "original_response", "created"]
        )),
        1234
    );
    assert_eq!(
        text(&nested(
            &projected,
            &[
                "_hidden_params",
                "original_response",
                "choices",
                "0",
                "finish_reason"
            ]
        )),
        "length"
    );

    // the async conversion keeps the raw SDK usage and the raw response dict
    let reply = buffered_reply(ClientKind::AsyncAzureOpenAi, TWO_CHOICES, true, &fixed);
    let projected = reply_json(&reply);
    let Json::Object(usage) = field(&projected, "usage") else {
        panic!("the raw usage object");
    };
    assert_eq!(usage.get("total_tokens"), Some(&Json::Number(5.into())));
    assert_eq!(
        usage.get("completion_tokens_details"),
        Some(&Json::Null),
        "the sdk dump keeps the null details"
    );
    assert_eq!(
        text(&nested(
            &projected,
            &["_hidden_params", "original_response", "system_fingerprint"]
        )),
        "fp_1"
    );
    assert_eq!(
        text(&nested(
            &projected,
            &[
                "_hidden_params",
                "original_response",
                "choices",
                "0",
                "finish_reason"
            ]
        )),
        "length"
    );

    // the async usage is the dict the SDK dumped, so it keeps keys the sync Usage folds away
    let reply = buffered_reply(ClientKind::AsyncAzureOpenAi, EXTRA_USAGE, true, &fixed);
    let projected = reply_json(&reply);
    assert_eq!(
        number(&nested(&projected, &["usage", "reasoning_tokens"])),
        2
    );
    assert_eq!(
        nested(&projected, &["usage", "completion_tokens_details"]),
        Json::Null
    );
    let reply = buffered_reply(ClientKind::AzureOpenAi, EXTRA_USAGE, false, &fixed);
    let projected = reply_json(&reply);
    let Json::Object(usage) = field(&projected, "usage") else {
        panic!("the sync usage object");
    };
    assert!(!usage.contains_key("reasoning_tokens"));
    assert_eq!(
        number(&nested(
            &projected,
            &["usage", "completion_tokens_details", "reasoning_tokens"]
        )),
        2
    );
    assert_eq!(
        number(&nested(
            &projected,
            &["usage", "completion_tokens_details", "text_tokens"]
        )),
        0
    );
    assert_eq!(
        nested(&projected, &["usage", "queue_time"]),
        Json::Number(serde_json::Number::from_f64(0.25).unwrap())
    );

    // a response without usage: the sync path zero-fills, the async path passes the explicit null on
    let reply = buffered_reply(ClientKind::AzureOpenAi, NO_USAGE, false, &fixed);
    let projected = reply_json(&reply);
    assert_eq!(number(&nested(&projected, &["usage", "total_tokens"])), 0);
    assert_eq!(
        text(&nested(&projected, &["choices", "0", "finish_reason"])),
        "content_filter"
    );
    let reply = buffered_reply(ClientKind::AsyncAzureOpenAi, NO_USAGE, true, &fixed);
    let projected = reply_json(&reply);
    let Json::Object(fields) = &projected else {
        panic!("a projected object");
    };
    assert_eq!(fields.get("usage"), Some(&Json::Null));
    let Json::Object(original) = nested(&projected, &["_hidden_params", "original_response"])
    else {
        panic!("the raw original response");
    };
    assert_eq!(original.get("usage"), Some(&Json::Null));

    // a buffered request answered with SSE fails instead of reading it as a completion
    let parts = Parts::new();
    let messages = text_messages();
    let mut params = object("{}");
    let mut hooks = super::recorder();
    let error = completion(
        ScriptedSource::new(
            ClientKind::AzureOpenAi,
            Answer::Stream(super::support::stream_items()),
        ),
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap_err();
    assert_eq!(error.status, Some(500));
    assert_eq!(error.message, "the answer is a stream, not a completion");
    assert_eq!(hooks.pre.len(), 1);
    assert!(hooks.post.is_empty());

    let mut params = object("{}");
    let mut hooks = super::recorder();
    let error = completion(
        ScriptedSource::new(
            ClientKind::AsyncAzureOpenAi,
            Answer::Stream(super::support::stream_items()),
        ),
        parts.call(Some(&messages), &mut params, true),
        &fixed,
        &mut hooks,
    )
    .unwrap_err();
    assert_eq!(error.status, Some(500));
    assert_eq!(error.message, "the answer is a stream, not a completion");
    assert_eq!(hooks.pre.len(), 1);
}
