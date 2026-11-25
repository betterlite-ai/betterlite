#![cfg(test)]

use super::{
    CompletionPrompt, ConvertError, ModerationArguments, TextArguments, TextCallWire, body,
    chat_messages, completion_request, moderation_model,
};
use crate::moderation::error::{CallFacts, CallFailure, convert_failure};
use crate::moderation::session::Entry;
use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_openai::moderation_request::ModerationInput;
use betterlite_openai::text_completion_request::PromptRefusal;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;

fn arguments(text: &str) -> TextArguments {
    let wire: TextCallWire = serde_json::from_str(&format!(r#"{{"arguments":{text}}}"#)).unwrap();
    wire.arguments
}

fn encoded<T>(value: &T) -> String
where
    T: serde::Serialize,
{
    serde_json::to_string(value).unwrap()
}

fn completion_prompt(arguments: &TextArguments) -> CompletionPrompt {
    CompletionPrompt::from(serde_json::to_value(&arguments.prompt).unwrap())
}

#[test]
fn request_string_prompt_is_one_user_message() {
    let call = arguments(r#"{"prompt": "hello"}"#);
    let messages = chat_messages(&call, false, "openai").unwrap();
    assert_eq!(encoded(&messages), r#"[{"content":"hello","role":"user"}]"#);
}

#[test]
fn request_string_list_prompt_is_one_message_per_string() {
    let call = arguments(r#"{"prompt": ["first", "second"]}"#);
    let messages = chat_messages(&call, false, "openai").unwrap();
    assert_eq!(
        encoded(&messages),
        r#"[{"content":"first","role":"user"},{"content":"second","role":"user"}]"#
    );
}

#[test]
fn request_token_prompt_is_refused_for_a_chat_provider() {
    let call = arguments(r#"{"prompt": [1, 2]}"#);
    assert!(matches!(
        chat_messages(&call, false, "text-completion-openai"),
        Err(ConvertError::Prompt)
    ));
    // the mock answers on a text provider, and the chat entry reads the token prompt as its JSON text
    let mocked = chat_messages(&call, true, "text-completion-openai").unwrap();
    assert_eq!(encoded(&mocked), r#"[{"content":"[1,2]","role":"user"}]"#);
    assert!(matches!(
        completion_prompt(&call),
        CompletionPrompt::Tokens(_)
    ));
}

#[rstest]
#[case::openai("openai")]
#[case::azure("azure")]
#[case::azure_text("azure_text")]
#[case::text_completion_codestral("text-completion-codestral")]
#[case::text_completion_openai("text-completion-openai")]
fn request_token_prompt_provider_covers_the_five_litellm_providers(#[case] provider: &str) {
    let call = arguments(r#"{"prompt": [1, 2]}"#);
    chat_messages(&call, true, provider).unwrap();
}

#[rstest]
#[case::anthropic("anthropic")]
#[case::groq("groq")]
#[case::text_completion_inception("text-completion-inception")]
fn request_token_prompt_refuses_another_provider(#[case] provider: &str) {
    let call = arguments(r#"{"prompt": [1, 2]}"#);
    assert!(
        matches!(
            chat_messages(&call, true, provider),
            Err(ConvertError::Prompt)
        ),
        "{provider}"
    );
}

#[test]
fn request_token_prompt_stays_tokens_for_the_text_endpoint() {
    let call = arguments(r#"{"prompt": [1, 2]}"#);
    let request = completion_request(&call).unwrap();
    assert_eq!(request.prompt, CompletionPrompt::Tokens(vec![1, 2]));
}

#[test]
fn request_unmapped_prompt_is_refused() {
    let unmapped = arguments(r#"{"prompt": 7}"#);
    assert!(matches!(
        completion_prompt(&unmapped).refusal(),
        Some(PromptRefusal::Unmapped)
    ));
    let empty = arguments(r#"{"prompt": []}"#);
    assert!(matches!(
        completion_prompt(&empty).refusal(),
        Some(PromptRefusal::Unmapped)
    ));
    let one_string = arguments(r#"{"prompt": ["only"]}"#);
    assert!(!matches!(
        completion_prompt(&one_string).refusal(),
        Some(PromptRefusal::Unmapped)
    ));
    let tokens = arguments(r#"{"prompt": [1, 2]}"#);
    assert!(!matches!(
        completion_prompt(&tokens).refusal(),
        Some(PromptRefusal::Unmapped)
    ));
}

#[test]
fn request_body_carries_the_provider_parameters() {
    let call = arguments(
        r#"{"prompt": "hello", "stream": false, "echo": true, "best_of": 2, "suffix": "end", "seed": 7, "metadata": {"a": 1}}"#,
    );
    let built = body(&call).unwrap();
    let text = encoded(&built);
    assert!(text.contains(r#""echo":true"#));
    assert!(text.contains(r#""best_of":2"#));
    assert!(text.contains(r#""suffix":"end""#));
    assert!(text.contains(r#""seed":7"#));
    assert!(text.contains(r#""stream":false"#));
    assert!(!text.contains("metadata"));
}

#[test]
fn request_unknown_parameter_is_refused() {
    let call = arguments(r#"{"prompt": "hello", "api_type": "openai"}"#);
    let refusal = body(&call).unwrap_err();
    assert!(refusal.to_string().contains("api_type"));
}

#[test]
fn request_num_retries_becomes_max_retries() {
    let call = arguments(r#"{"prompt": "hello", "num_retries": 4}"#);
    let built = body(&call).unwrap();
    let text = encoded(&built);
    assert!(text.contains(r#""max_retries":4"#));
    assert!(!text.contains("num_retries"));
}

#[test]
fn request_num_retries_wins_over_max_retries() {
    let after = arguments(r#"{"prompt": "hello", "max_retries": 1, "num_retries": 4}"#);
    assert!(encoded(&body(&after).unwrap()).contains(r#""max_retries":4"#));
    let before = arguments(r#"{"prompt": "hello", "num_retries": 4, "max_retries": 1}"#);
    assert!(encoded(&body(&before).unwrap()).contains(r#""max_retries":4"#));
}

#[test]
fn request_max_retries_alone_is_kept() {
    let call = arguments(r#"{"prompt": "hello", "max_retries": 3}"#);
    assert!(encoded(&body(&call).unwrap()).contains(r#""max_retries":3"#));
}

#[test]
fn request_moderation_model_defaults_and_strips_the_openai_prefix_async() {
    let call: ModerationArguments = serde_json::from_str(r#"{"input": "hello"}"#).unwrap();
    assert_eq!(
        moderation_model(&call, Entry::Sync),
        "omni-moderation-latest"
    );
    let prefixed: ModerationArguments =
        serde_json::from_str(r#"{"input": "hello", "model": "openai/omni-moderation-latest"}"#)
            .unwrap();
    assert_eq!(
        moderation_model(&prefixed, Entry::Sync),
        "openai/omni-moderation-latest"
    );
    assert_eq!(
        moderation_model(&prefixed, Entry::Async),
        "omni-moderation-latest"
    );
}

#[test]
fn request_moderation_input_reads_text_and_texts() {
    let text: ModerationArguments = serde_json::from_str(r#"{"input": "hello"}"#).unwrap();
    assert_eq!(text.input, ModerationInput::Text(String::from("hello")));
    let texts: ModerationArguments = serde_json::from_str(r#"{"input": ["one", "two"]}"#).unwrap();
    assert_eq!(
        texts.input,
        ModerationInput::Texts(vec![String::from("one"), String::from("two")])
    );
}

#[test]
fn error_unsupported_parameter_names_the_class() {
    let call = arguments(r#"{"prompt": "hello", "api_type": "openai"}"#);
    let failure = convert_failure(
        &body(&call).unwrap_err(),
        &CallFacts {
            model: "gpt-3.5-turbo-instruct",
            provider: "text-completion-openai",
        },
    );
    let payload = super::NativeTextErrorPayload::from_class(&failure.0);
    assert_eq!(payload.kind, "UnsupportedParamsError");
    assert_eq!(payload.status_code, Some(400));
    assert!(payload.message.contains("api_type"));
}

#[test]
fn error_payload_names_the_class_and_its_status() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let class = dispatch(
        &DispatchInput::Raw(Box::new(RawException {
            model: String::from("gpt-4o"),
            custom_llm_provider: String::from("openai"),
            error_str: String::from("Invalid API key provided"),
            status_code: Some(401),
            status_code_is_synthesized: false,
            message: None,
            body: None,
            exception_type: String::new(),
        })),
        &redactor,
    );
    let payload = super::NativeTextErrorPayload::from_class(&class);
    assert_eq!(payload.kind, "AuthenticationError");
    assert_eq!(payload.status_code, Some(401));
    assert_eq!(payload.llm_provider.as_deref(), Some("openai"));
    assert_eq!(payload.model.as_deref(), Some("gpt-4o"));
    assert!(payload.message.contains("Invalid API key"));
}

#[test]
fn error_payload_carries_no_status_for_a_plain_exception() {
    let failure = convert_failure(
        &ConvertError::Prompt,
        &CallFacts {
            model: "gpt-4o",
            provider: "openai",
        },
    );
    let payload = super::NativeTextErrorPayload::from_class(&failure.0);
    assert_eq!(payload.kind, "Exception");
    assert!(payload.status_code.is_none());
    assert!(payload.message.starts_with("Unmapped prompt format"));
}

#[test]
fn error_redaction_failure_is_an_internal_server() {
    let Err(redaction) = Redactor::new(usize::MAX) else {
        panic!("an impossible repetition count must not compile");
    };
    let payload = super::NativeTextErrorPayload::from_class(&CallFailure::from(redaction).0);
    assert_eq!(payload.kind, "InternalServerError");
    assert_eq!(payload.status_code, Some(500));
}
