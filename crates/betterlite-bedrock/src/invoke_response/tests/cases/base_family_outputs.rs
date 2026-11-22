#![cfg(test)]

use crate::invoke_response::tests::support::{CREATED, decode, fixture, headers};

pub(in crate::invoke_response::tests) fn base_family_outputs() {
    titan_output();
    cohere_text_key_wins();
    cohere_generations_map_finish_reason();
    ai21_text();
    generation_text();
    mistral_outputs();
    mistral_choices();
    every_other_provider_is_titan();
}

fn titan_output() {
    let mut case = fixture();
    case.headers = headers("11", "4");
    let run = decode(
        &case,
        r#"{"results": [{"outputText": "the answer", "tokenCount": 9}]}"#,
    );
    assert!(run.calls.is_empty(), "the headers own the usage");
    let response = run.chat();
    assert_eq!(response.id, "chatcmpl-supplied");
    assert_eq!(response.choices.len(), 1);
    assert_eq!(response.choices[0].index, 0);
    assert_eq!(
        response.choices[0].message.content.as_deref(),
        Some("the answer")
    );
    assert_eq!(response.choices[0].finish_reason, None);
    assert_eq!(response.usage.prompt_tokens, 11);
    assert_eq!(response.usage.completion_tokens, 4);
    assert_eq!(response.usage.total_tokens, 15);
    assert_eq!(response.created, CREATED);
    assert_eq!(response.model, case.model);
}

fn cohere_text_key_wins() {
    let mut case = fixture();
    case.model = String::from("cohere.command-r-v1:0");
    let run = decode(
        &case,
        r#"{"text": "direct", "generations": [{"text": "ignored", "finish_reason": "COMPLETE"}]}"#,
    );
    let response = run.chat();
    assert_eq!(
        response.choices[0].message.content.as_deref(),
        Some("direct")
    );
    assert_eq!(response.choices[0].finish_reason, None);
}

fn cohere_generations_map_finish_reason() {
    let mut case = fixture();
    case.model = String::from("cohere.command-r-v1:0");
    let run = decode(
        &case,
        r#"{"generations": [{"text": "cohere says", "finish_reason": "MAX_TOKENS"}]}"#,
    );
    let response = run.chat();
    assert_eq!(
        response.choices[0].message.content.as_deref(),
        Some("cohere says")
    );
    assert_eq!(response.choices[0].finish_reason.as_deref(), Some("length"));
}

fn ai21_text() {
    let mut case = fixture();
    case.model = String::from("ai21.j2-mid-v1");
    let run = decode(
        &case,
        r#"{"completions": [{"data": {"text": "ai21 says"}}]}"#,
    );
    assert_eq!(
        run.chat().choices[0].message.content.as_deref(),
        Some("ai21 says")
    );
}

fn generation_text() {
    for model in [
        "meta.llama3-70b-instruct-v1:0",
        "llama.3-8b",
        "deepseek_r1.deepseek-r1-v1:0",
    ] {
        let mut case = fixture();
        case.model = model.to_owned();
        let run = decode(&case, r#"{"generation": "generated text"}"#);
        assert_eq!(
            run.chat().choices[0].message.content.as_deref(),
            Some("generated text"),
            "{model}"
        );
    }
}

fn mistral_outputs() {
    let mut case = fixture();
    case.model = String::from("mistral.mistral-large-2402-v1:0");
    let run = decode(
        &case,
        r#"{"outputs": [{"text": "mistral says", "stop_reason": "stop"}]}"#,
    );
    let response = run.chat();
    assert_eq!(
        response.choices[0].message.content.as_deref(),
        Some("mistral says")
    );
    assert_eq!(response.choices[0].finish_reason.as_deref(), Some("stop"));
}

fn mistral_choices() {
    let mut case = fixture();
    case.model = String::from("mistral.mistral-large-2402-v1:0");
    let run = decode(
        &case,
        r#"{"choices": [{"message": {"content": "chat text"}, "finish_reason": "length"}]}"#,
    );
    let response = run.chat();
    assert_eq!(
        response.choices[0].message.content.as_deref(),
        Some("chat text")
    );
    assert_eq!(response.choices[0].finish_reason.as_deref(), Some("length"));
}

fn every_other_provider_is_titan() {
    for model in [
        "amazon.titan-text-premier-v1:0",
        "qwen.qwen3-32b-v1:0",
        "qwen.qwen2-72b-instruct",
        "openai.gpt-oss-20b-1:0",
        "stability.stable-image-core-v1:1",
        "moonshot.kimi-k2",
        "vendor.unknown-model",
    ] {
        let mut case = fixture();
        case.model = model.to_owned();
        let run = decode(&case, r#"{"results": [{"outputText": "titan fallback"}]}"#);
        assert_eq!(
            run.chat().choices[0].message.content.as_deref(),
            Some("titan fallback"),
            "{model}"
        );
    }
}
