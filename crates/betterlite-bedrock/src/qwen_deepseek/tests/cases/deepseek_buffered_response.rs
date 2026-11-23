#![cfg(test)]

use crate::qwen_deepseek::error::DeepseekReplyError;
use crate::qwen_deepseek::reply::{Reply, Usage};
use crate::qwen_deepseek::tests::counter::CounterCall;
use crate::qwen_deepseek::tests::support::{
    CREATED, deepseek, deepseek_fixture, headers, object, seeded, two_choices, with_tool_calls,
};

pub(in crate::qwen_deepseek::tests) fn deepseek_buffered_response() {
    seed_defaults();
    base_reply_conversion();
    header_usage_replaces_the_counter();
    split_reasoning_and_content();
    empty_reasoning_keeps_the_key();
    only_the_first_choice_is_processed();
    trigger_conditions();
    tool_call_content_skips_the_split();
    prompt_errors();
    base_errors_pass_through();
}

// ModelResponse names the chat object even when the wire seed omits it.
fn seed_defaults() {
    let reply: Reply =
        serde_json::from_str(r#"{"id": "chatcmpl-empty"}"#).expect("the seed parses");
    assert_eq!(
        reply.object, "chat.completion",
        "an absent object takes the chat completion default"
    );
}

fn base_reply_conversion() {
    let mut case = deepseek_fixture();
    case.request_data = object(r#"{"prompt": "hello"}"#);
    let run = deepseek(&case, r#"{"generation": "the answer"}"#);
    let reply = run.reply();
    assert_eq!(reply.id, "chatcmpl-supplied", "the id is kept");
    assert_eq!(reply.object, "chat.completion");
    assert_eq!(reply.created, CREATED, "the clock sets the creation time");
    assert_eq!(reply.model, case.model);
    assert_eq!(
        reply.choices[0].message.content.as_deref(),
        Some("the answer")
    );
    assert_eq!(reply.choices[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(
        reply.usage,
        Some(Usage {
            prompt_tokens: 7,
            completion_tokens: 3,
            total_tokens: 10,
        }),
        "the counter supplies both counts"
    );
    assert_eq!(run.calls.len(), 2, "one prompt and one completion count");
    assert!(matches!(run.calls.first(), Some(CounterCall::Messages(_))));
    assert!(
        matches!(run.calls.get(1), Some(CounterCall::Text(text)) if text.as_str() == "the answer"),
        "the base counts the produced content"
    );
}

fn header_usage_replaces_the_counter() {
    let mut case = deepseek_fixture();
    case.headers = headers("11", "4");
    case.request_data = object(r#"{"prompt": "no trigger here"}"#);
    let run = deepseek(&case, r#"{"generation": "the answer"}"#);
    assert_eq!(
        run.reply().usage,
        Some(Usage {
            prompt_tokens: 11,
            completion_tokens: 4,
            total_tokens: 15,
        })
    );
    assert!(run.calls.is_empty(), "the headers own the usage");
    assert_eq!(
        run.reply().choices[0].message.provider_specific_fields,
        None,
        "an untriggered reply gains no reasoning field"
    );
}

fn split_reasoning_and_content() {
    let mut case = deepseek_fixture();
    case.request_data = object(r#"{"prompt": "solve <think>"}"#);
    let run = deepseek(
        &case,
        r#"{"generation": "reasoning text</think>final answer"}"#,
    );
    let reply = run.reply();
    assert_eq!(
        reply.choices[0].message.content.as_deref(),
        Some("final answer")
    );
    assert_eq!(
        reply.choices[0].message.provider_specific_fields,
        Some(object(r#"{"reasoning_content": "reasoning text"}"#))
    );

    let mut merged = deepseek_fixture();
    merged.request_data = object(r#"{"prompt": "solve<think>"}"#);
    merged.response.choices[0].message.provider_specific_fields =
        Some(object(r#"{"existing": "kept"}"#));
    let run = deepseek(&merged, r#"{"generation": "thinking</think>answer"}"#);
    assert_eq!(
        run.reply().choices[0].message.provider_specific_fields,
        Some(object(
            r#"{"existing": "kept", "reasoning_content": "thinking"}"#
        )),
        "a truthy reasoning merges into the existing fields"
    );

    let run = deepseek(&case, r#"{"generation": "plain answer"}"#);
    assert_eq!(
        run.reply().choices[0].message.content.as_deref(),
        Some("<think>plain answer"),
        "the prepended token stays without a closing tag"
    );
    assert_eq!(
        run.reply().choices[0].message.provider_specific_fields,
        None
    );
}

fn empty_reasoning_keeps_the_key() {
    let mut case = deepseek_fixture();
    case.request_data = object(r#"{"prompt": "go<think>"}"#);
    let run = deepseek(&case, r#"{"generation": "</think>tail"}"#);
    assert_eq!(
        run.reply().choices[0].message.content.as_deref(),
        Some("tail")
    );
    assert_eq!(
        run.reply().choices[0].message.provider_specific_fields,
        None,
        "an empty reasoning sets no key"
    );

    let run = deepseek(&case, r#"{"generation": "thought</think>"}"#);
    assert_eq!(
        run.reply().choices[0].message.content.as_deref(),
        Some(""),
        "the content after the closing tag can be empty"
    );
    assert_eq!(
        run.reply().choices[0].message.provider_specific_fields,
        Some(object(r#"{"reasoning_content": "thought"}"#))
    );
}

fn only_the_first_choice_is_processed() {
    let mut case = deepseek_fixture();
    case.response = two_choices();
    case.request_data = object(r#"{"prompt": "go<think>"}"#);
    let run = deepseek(&case, r#"{"generation": "think</think>done"}"#);
    let reply = run.reply();
    assert_eq!(
        reply.choices[0].message.content.as_deref(),
        Some("done"),
        "the first choice carries the parsed content"
    );
    assert_eq!(
        reply.choices[0].message.provider_specific_fields,
        Some(object(r#"{"reasoning_content": "think"}"#))
    );
    assert_eq!(
        reply.choices[1].message.content.as_deref(),
        Some("second"),
        "later choices keep their content"
    );
    assert_eq!(reply.choices[1].message.provider_specific_fields, None);
}

fn trigger_conditions() {
    let body = r#"{"generation": "answer</think>rest"}"#;
    for value in [
        r#""""#,
        r#""   ""#,
        r#""hello""#,
        r#""hello<think> extra""#,
        r#""<think>x""#,
        "null",
        "0",
        "false",
        "[]",
        "{}",
    ] {
        let mut case = deepseek_fixture();
        case.request_data = object(&format!(r#"{{"prompt": {value}}}"#));
        let run = deepseek(&case, body);
        assert_eq!(
            run.reply().choices[0].message.content.as_deref(),
            Some("answer</think>rest"),
            "prompt {value} does not trigger"
        );
        assert_eq!(
            run.reply().choices[0].message.provider_specific_fields,
            None
        );
    }
    let mut missing = deepseek_fixture();
    missing.request_data = object(r#"{"model": "amazon.deepseek-r1"}"#);
    let run = deepseek(&missing, body);
    assert_eq!(
        run.reply().choices[0].message.content.as_deref(),
        Some("answer</think>rest"),
        "a missing prompt does not trigger"
    );
    for value in [
        r#""hello<think>""#,
        r#""hello<think> ""#,
        r#""hello<think>\n\t""#,
        r#""\u001c<think>\u001f""#,
        r#""  <think>""#,
    ] {
        let mut case = deepseek_fixture();
        case.request_data = object(&format!(r#"{{"prompt": {value}}}"#));
        let run = deepseek(&case, body);
        assert_eq!(
            run.reply().choices[0].message.content.as_deref(),
            Some("rest"),
            "prompt {value} triggers"
        );
        assert_eq!(
            run.reply().choices[0].message.provider_specific_fields,
            Some(object(r#"{"reasoning_content": "answer"}"#)),
            "prompt {value}"
        );
    }
}

fn tool_call_content_skips_the_split() {
    let mut case = deepseek_fixture();
    case.response = with_tool_calls(seeded());
    case.headers = headers("4", "2");
    case.request_data = object(r#"{"prompt": "go<think>"}"#);
    let run = deepseek(&case, r#"{"generation": "ignored"}"#);
    let reply = run.reply();
    assert_eq!(
        reply.choices[0].message.content, None,
        "a tool-call message keeps its seeded content"
    );
    assert_eq!(reply.choices[0].message.provider_specific_fields, None);
    assert_eq!(
        reply.choices[0].message.tool_calls.as_ref().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        reply.usage,
        Some(Usage {
            prompt_tokens: 4,
            completion_tokens: 2,
            total_tokens: 6,
        })
    );
}

fn prompt_errors() {
    for value in ["5", "true", "[1]", r#"{"a": 1}"#] {
        let mut case = deepseek_fixture();
        case.request_data = object(&format!(r#"{{"prompt": {value}}}"#));
        let run = deepseek(&case, r#"{"generation": "answer"}"#);
        assert!(
            matches!(run.error(), DeepseekReplyError::PromptNotText),
            "prompt {value}"
        );
    }
}

fn base_errors_pass_through() {
    let case = deepseek_fixture();
    let invalid = deepseek(&case, "not json");
    assert!(matches!(invalid.error(), DeepseekReplyError::Base(_)));
    assert_eq!(invalid.bedrock().error.status_code, 200);
    assert_eq!(invalid.bedrock().error.message, "not json");
    let missing = deepseek(&case, r#"{"text": "x"}"#);
    assert_eq!(missing.bedrock().error.status_code, 422);
    assert!(
        missing
            .bedrock()
            .error
            .message
            .starts_with("Error processing="),
        "{}",
        missing.bedrock().error.message
    );
    let not_text = deepseek(&case, r#"{"generation": 5}"#);
    assert_eq!(not_text.bedrock().error.status_code, 200);
    let mut failed = deepseek_fixture();
    failed.status = 400;
    let status = deepseek(&failed, "not json");
    assert_eq!(status.bedrock().error.status_code, 400);
}
