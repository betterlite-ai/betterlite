use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use rstest::rstest;

use crate::count::default_encoding::{DefaultEncoding, DefaultEngine};
use crate::count::settings::Settings;
use crate::count::wire::CustomTokenizerWire;
use crate::engine::Engine;
use crate::engine::assets;
use crate::engine::error::EngineError;
use crate::engine::selection::SelectionError;
use crate::engine::tiktoken::{Encoding, TiktokenEngine};

use super::support;

const TOOLS: &str = r#"[{"type":"function","function":{"name":"get_weather","description":"Get weather","parameters":{"type":"object","properties":{"city":{"type":"string","description":"City name"},"days":{"type":"integer"}},"required":["city"]}}}]"#;
const TOKENIZER_JSON: &str = r#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,"pre_tokenizer":{"type":"Whitespace"},"post_processor":null,"decoder":null,"model":{"type":"WordLevel","vocab":{"[UNK]":0,"hello":1,"world":2,"hi":3},"unk_token":"[UNK]"}}"#;

#[derive(Debug)]
enum Expected {
    Tokens(usize),
    Disabled(usize),
    Error(&'static str),
    Fails,
}

#[rstest]
#[case::text_known_model(
    r#"{"model":"gpt-3.5-turbo","text":"hello world"}"#,
    4_000_000,
    1024,
    Expected::Tokens(2)
)]
#[case::text_o200k_model(
    "{\"model\":\"gpt-4o\",\"text\":\"hello world \u{4e16}\u{754c}\"}",
    4_000_000,
    1024,
    Expected::Tokens(3)
)]
#[case::text_cl100k_model(
    "{\"model\":\"gpt-3.5-turbo\",\"text\":\"hello world \u{4e16}\u{754c}\"}",
    4_000_000,
    1024,
    Expected::Tokens(6)
)]
#[case::text_unknown_model_falls_back_to_cl100k(
    "{\"model\":\"totally-unknown\",\"text\":\"hello world \u{4e16}\u{754c}\"}",
    4_000_000,
    1024,
    Expected::Tokens(6)
)]
#[case::text_empty_model_falls_back_to_cl100k(
    r#"{"text":"hello world"}"#,
    4_000_000,
    1024,
    Expected::Tokens(2)
)]
#[case::text_p50k_model(
    r#"{"model":"text-davinci-003","text":"hello world"}"#,
    4_000_000,
    1024,
    Expected::Tokens(2)
)]
#[case::text_anthropic_model_counts_directly(
    "{\"model\":\"claude-haiku-4-5\",\"text\":\"hello world \u{4e16}\u{754c}\"}",
    4_000_000,
    4,
    Expected::Tokens(6)
)]
#[case::text_list_is_joined(
    r#"{"model":"gpt-3.5-turbo","text":["hello"," world"]}"#,
    4_000_000,
    1024,
    Expected::Tokens(2)
)]
#[case::text_empty(
    r#"{"model":"gpt-3.5-turbo","text":""}"#,
    4_000_000,
    1024,
    Expected::Tokens(0)
)]
#[case::text_empty_list(
    r#"{"model":"gpt-3.5-turbo","text":[]}"#,
    4_000_000,
    1024,
    Expected::Tokens(0)
)]
#[case::text_unicode(
    "{\"model\":\"gpt-3.5-turbo\",\"text\":\"H\u{e9}llo, \u{4e16}\u{754c}! \u{1f44b}\"}",
    4_000_000,
    1024,
    Expected::Tokens(11)
)]
#[case::text_with_empty_tools(
    r#"{"model":"gpt-3.5-turbo","text":"hello world","tools":[]}"#,
    4_000_000,
    1024,
    Expected::Tokens(2)
)]
#[case::text_and_messages(
    r#"{"model":"gpt-3.5-turbo","text":"x","messages":[]}"#,
    4_000_000,
    1024,
    Expected::Error("text and messages cannot both be set")
)]
#[case::no_text_and_no_messages(
    r#"{"model":"gpt-3.5-turbo"}"#,
    4_000_000,
    1024,
    Expected::Error("either text or messages must be provided")
)]
#[case::text_with_tools(
    r#"{"model":"gpt-3.5-turbo","text":"x","tools":[{"type":"function","function":{"name":"f"}}]}"#,
    4_000_000,
    1024,
    Expected::Error("tools or tool_choice cannot be set when counting text")
)]
#[case::text_with_tool_choice(
    r#"{"model":"gpt-3.5-turbo","text":"x","tool_choice":"auto"}"#,
    4_000_000,
    1024,
    Expected::Error("tools or tool_choice cannot be set when counting text")
)]
#[case::chunked_text_counts_each_chunk(
    r#"{"model":"gpt-3.5-turbo","text":"hello world"}"#,
    4_000_000,
    4,
    Expected::Tokens(5)
)]
#[case::chunked_ascii_text(
    r#"{"model":"gpt-3.5-turbo","text":"abcdefghijabcdefghijabcdefghijabcdefghij"}"#,
    4_000_000,
    4,
    Expected::Tokens(18)
)]
#[case::chunked_text_by_one_scalar(
    r#"{"model":"gpt-3.5-turbo","text":"hello world"}"#,
    4_000_000,
    1,
    Expected::Tokens(11)
)]
#[case::chunked_unicode_by_one_scalar(
    "{\"model\":\"gpt-3.5-turbo\",\"text\":\"H\u{e9}llo, \u{4e16}\u{754c}! \u{1f44b}\"}",
    4_000_000,
    1,
    Expected::Tokens(15)
)]
#[case::chunked_o200k_by_one_scalar(
    "{\"model\":\"gpt-4o\",\"text\":\"hello world \u{4e16}\u{754c}\"}",
    4_000_000,
    1,
    Expected::Tokens(14)
)]
#[case::sampled_tie_rounds_down_to_even(
    r#"{"model":"gpt-3.5-turbo","text":"ababababababababababababababababa"}"#,
    32,
    1024,
    Expected::Tokens(16)
)]
#[case::sampled_tie_rounds_up_to_even(
    r#"{"model":"gpt-3.5-turbo","text":"abababababababababababababababababa"}"#,
    32,
    1024,
    Expected::Tokens(18)
)]
#[case::sampled_long_text(r#"{"model":"gpt-3.5-turbo","text":"hello world hello world hello world hello world hello world hello world hello world hello world hello world hello world "}"#, 32, 1024, Expected::Tokens(71))]
#[case::exact_at_threshold(
    "{\"model\":\"gpt-3.5-turbo\",\"text\":\"H\u{e9}llo, \u{4e16}\u{754c}! \u{1f44b} the\"}",
    16,
    4,
    Expected::Tokens(13)
)]
#[case::sampled_one_over_threshold(
    "{\"model\":\"gpt-3.5-turbo\",\"text\":\"H\u{e9}llo, \u{4e16}\u{754c}! \u{1f44b} the \"}",
    16,
    4,
    Expected::Tokens(20)
)]
#[case::sampled_above_threshold(
    "{\"model\":\"gpt-3.5-turbo\",\"text\":\"H\u{e9}llo, \u{4e16}\u{754c}! \u{1f44b} the quick brown fox\"}",
    16,
    4,
    Expected::Tokens(32)
)]
#[case::exact_short_unicode(
    "{\"model\":\"gpt-3.5-turbo\",\"text\":\"H\u{e9}llo, \u{4e16}\u{754c}!\"}",
    16,
    4,
    Expected::Tokens(10)
)]
#[case::messages_empty_counts_the_reply_priming(
    r#"{"model":"gpt-3.5-turbo","messages":[]}"#,
    4_000_000,
    1024,
    Expected::Tokens(3)
)]
#[case::messages_simple(
    r#"{"model":"gpt-3.5-turbo","messages":[{"role":"user","content":"hello world"}]}"#,
    4_000_000,
    1024,
    Expected::Tokens(9)
)]
#[case::messages_system(r#"{"model":"gpt-3.5-turbo","messages":[{"role":"system","content":"s"},{"role":"user","content":"hello world"}]}"#, 4_000_000, 1024, Expected::Tokens(14))]
#[case::messages_content_null_is_skipped(
    r#"{"model":"gpt-3.5-turbo","messages":[{"role":"user","content":null}]}"#,
    4_000_000,
    1024,
    Expected::Tokens(7)
)]
#[case::messages_name_modern(r#"{"model":"gpt-3.5-turbo","messages":[{"role":"user","content":"hello world","name":"bob"}]}"#, 4_000_000, 1024, Expected::Tokens(11))]
#[case::messages_name_on_turbo_0301(r#"{"model":"gpt-3.5-turbo-0301","messages":[{"role":"user","content":"hello world","name":"bob"}]}"#, 4_000_000, 1024, Expected::Tokens(11))]
#[case::messages_two_named_messages(r#"{"model":"gpt-3.5-turbo-0301","messages":[{"role":"user","content":"hi","name":"bob"},{"role":"user","content":"hi","name":"ann"}]}"#, 4_000_000, 1024, Expected::Tokens(17))]
#[case::messages_response_count_skips_the_extra(r#"{"model":"gpt-3.5-turbo","messages":[{"role":"user","content":"hello world"}],"count_response_tokens":true}"#, 4_000_000, 1024, Expected::Tokens(6))]
#[case::messages_tool_calls(r#"{"model":"gpt-3.5-turbo","messages":[{"role":"assistant","tool_calls":[{"id":"1","type":"function","function":{"name":"f","arguments":"{\"x\": 1}"}}]}]}"#, 4_000_000, 1024, Expected::Tokens(13))]
#[case::messages_function_call(r#"{"model":"gpt-3.5-turbo","messages":[{"role":"assistant","function_call":{"name":"f","arguments":"{\"y\": 2}"}}]}"#, 4_000_000, 1024, Expected::Tokens(13))]
#[case::messages_tool_call_without_function_fails(r#"{"model":"gpt-3.5-turbo","messages":[{"role":"assistant","tool_calls":[{"id":"1","type":"function"}]}]}"#, 4_000_000, 1024, Expected::Fails)]
#[case::messages_tool_calls_not_a_list_fails(
    r#"{"model":"gpt-3.5-turbo","messages":[{"role":"assistant","tool_calls":{"a":1}}]}"#,
    4_000_000,
    1024,
    Expected::Fails
)]
#[case::messages_tool_call_id(r#"{"model":"gpt-3.5-turbo","messages":[{"role":"tool","content":"ok","tool_call_id":"call_abc"}]}"#, 4_000_000, 1024, Expected::Tokens(11))]
#[case::messages_passthrough_string_counts(
    r#"{"model":"gpt-3.5-turbo","messages":[{"role":"user","content":"hi","custom":"ignored?"}]}"#,
    4_000_000,
    1024,
    Expected::Tokens(10)
)]
#[case::messages_passthrough_number_is_skipped(
    r#"{"model":"gpt-3.5-turbo","messages":[{"role":"user","content":"hi","custom":5}]}"#,
    4_000_000,
    1024,
    Expected::Tokens(8)
)]
#[case::messages_reasoning_content(r#"{"model":"gpt-3.5-turbo","messages":[{"role":"assistant","content":"hi","reasoning_content":"thought"}]}"#, 4_000_000, 1024, Expected::Tokens(9))]
#[case::messages_text_block(r#"{"model":"gpt-3.5-turbo","messages":[{"role":"user","content":[{"type":"text","text":"hello world"}]}]}"#, 4_000_000, 1024, Expected::Tokens(9))]
#[case::messages_string_parts(
    r#"{"model":"gpt-3.5-turbo","messages":[{"role":"user","content":["hello"," world"]}]}"#,
    4_000_000,
    1024,
    Expected::Tokens(9)
)]
#[case::messages_text_block_with_chunks(r#"{"model":"gpt-3.5-turbo","messages":[{"role":"user","content":[{"type":"text","text":"hello world"}]}]}"#, 4_000_000, 4, Expected::Tokens(12))]
#[case::messages_image_block_is_b197(r#"{"model":"gpt-3.5-turbo","messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.com/a.png"}}]}]}"#, 4_000_000, 1024, Expected::Error("content block type image_url is not counted"))]
#[case::messages_search_results(r#"{"model":"gpt-3.5-turbo","messages":[{"role":"tool","search_results":[{"source":"src","title":"title","content":[{"text":"body"}],"citations":[1,2]}]}]}"#, 4_000_000, 1024, Expected::Tokens(16))]
#[case::messages_search_results_without_citations(r#"{"model":"gpt-3.5-turbo","messages":[{"role":"tool","search_results":[{"source":"src","title":"title","content":[{"text":"body"}]}]}]}"#, 4_000_000, 1024, Expected::Tokens(11))]
#[case::messages_search_results_not_a_list_is_skipped(
    r#"{"model":"gpt-3.5-turbo","messages":[{"role":"tool","search_results":"nope"}]}"#,
    4_000_000,
    1024,
    Expected::Tokens(9)
)]
#[case::tools_add_the_formatted_definitions(
    r#"{"model":"gpt-3.5-turbo","messages":[{"role":"user","content":"hi"}],"tools":TOOLS}"#,
    4_000_000,
    1024,
    Expected::Tokens(51)
)]
#[case::tools_with_system_message_discount(r#"{"model":"gpt-3.5-turbo","messages":[{"role":"system","content":"s"},{"role":"user","content":"hi"}],"tools":TOOLS}"#, 4_000_000, 1024, Expected::Tokens(52))]
#[case::tools_choice_none(r#"{"model":"gpt-3.5-turbo","messages":[{"role":"user","content":"hi"}],"tools":TOOLS,"tool_choice":"none"}"#, 4_000_000, 1024, Expected::Tokens(52))]
#[case::tools_choice_auto(r#"{"model":"gpt-3.5-turbo","messages":[{"role":"user","content":"hi"}],"tools":TOOLS,"tool_choice":"auto"}"#, 4_000_000, 1024, Expected::Tokens(51))]
#[case::tools_choice_named(r#"{"model":"gpt-3.5-turbo","messages":[{"role":"user","content":"hi"}],"tools":TOOLS,"tool_choice":{"type":"function","function":{"name":"get_weather"}}}"#, 4_000_000, 1024, Expected::Tokens(60))]
#[case::tools_empty_list_adds_nothing(
    r#"{"model":"gpt-3.5-turbo","messages":[{"role":"user","content":"hi"}],"tools":[]}"#,
    4_000_000,
    1024,
    Expected::Tokens(8)
)]
#[case::choice_none_without_tools(
    r#"{"model":"gpt-3.5-turbo","messages":[{"role":"user","content":"hi"}],"tool_choice":"none"}"#,
    4_000_000,
    1024,
    Expected::Tokens(9)
)]
#[case::developer_message_does_not_discount_tools(r#"{"model":"gpt-3.5-turbo","messages":[{"role":"developer","content":"s"},{"role":"user","content":"hi"}],"tools":[{"type":"function","function":{"name":"f"}}]}"#, 4_000_000, 1024, Expected::Tokens(36))]
#[case::custom_openai_handle_by_tag(r#"{"model":"gpt-3.5-turbo","text":"hello world","custom_tokenizer":{"type":"openai_tokenizer"}}"#, 4_000_000, 1024, Expected::Tokens(2))]
#[case::custom_huggingface_handle_by_tag_fails_on_bad_json(r#"{"model":"gpt-4o","text":"hello world","custom_tokenizer":{"type":"huggingface_tokenizer","tokenizer_json":"{}"}}"#, 4_000_000, 1024, Expected::Fails)]
#[case::custom_huggingface_handle_by_unknown_tag_fails(
    r#"{"model":"gpt-4o","text":"hello world","custom_tokenizer":{"type":"other_tokenizer"}}"#,
    4_000_000,
    1024,
    Expected::Fails
)]
#[case::disabled_text(
    r#"{"model":"gpt-3.5-turbo","text":"hello world"}"#,
    4_000_000,
    1024,
    Expected::Disabled(0)
)]
#[case::disabled_text_and_messages(
    r#"{"model":"gpt-3.5-turbo","text":"x","messages":[]}"#,
    4_000_000,
    1024,
    Expected::Disabled(0)
)]
#[tokio::test]
async fn text_tokens_token_count_tests_behavior(
    #[case] request: &str,
    #[case] max_exact_chars: usize,
    #[case] chunk_chars: usize,
    #[case] expected: Expected,
) {
    let directory = tempfile::tempdir().unwrap();
    let disabled = matches!(expected, Expected::Disabled(_));
    let settings = support::settings(max_exact_chars, chunk_chars, disabled);
    let counter = support::counter(settings, directory.path());
    let prepared = request.replace("TOOLS", TOOLS);
    match expected {
        Expected::Tokens(tokens) | Expected::Disabled(tokens) => assert_eq!(
            support::count_request(&counter, &prepared).await.unwrap(),
            tokens
        ),
        Expected::Error(message) => assert_eq!(
            support::count_request(&counter, &prepared)
                .await
                .unwrap_err(),
            message
        ),
        Expected::Fails => {
            let _failure = support::count_request(&counter, &prepared)
                .await
                .unwrap_err();
        }
    }
}

#[tokio::test]
async fn custom_tokenizer_bypasses_model_selection() {
    let directory = tempfile::tempdir().unwrap();
    let settings = support::settings(4_000_000, 1, false);
    let counter = support::counter(settings, directory.path());
    let for_gpt = support::count_with_tokenizer(
        &counter,
        "gpt-4o",
        "hello world hello",
        CustomTokenizerWire::HuggingFace {
            tokenizer_json: TOKENIZER_JSON.to_owned(),
        },
    )
    .await
    .unwrap();
    assert_eq!(for_gpt, 3);
    let for_claude = support::count_with_tokenizer(
        &counter,
        "claude-2.1",
        "hello world hello",
        CustomTokenizerWire::HuggingFace {
            tokenizer_json: TOKENIZER_JSON.to_owned(),
        },
    )
    .await
    .unwrap();
    assert_eq!(for_claude, 3);
}

#[tokio::test]
async fn custom_openai_tokenizer_counts_with_the_model_encoding() {
    let directory = tempfile::tempdir().unwrap();
    let counter = support::counter(Settings::default(), directory.path());
    let from_cl100k_model = support::count_with_tokenizer(
        &counter,
        "gpt-3.5-turbo",
        "hello world \u{4e16}\u{754c}",
        CustomTokenizerWire::OpenAi,
    )
    .await
    .unwrap();
    assert_eq!(from_cl100k_model, 6);
    let from_o200k_model = support::count_with_tokenizer(
        &counter,
        "gpt-4o",
        "hello world \u{4e16}\u{754c}",
        CustomTokenizerWire::OpenAi,
    )
    .await
    .unwrap();
    assert_eq!(from_o200k_model, 3);
}

#[tokio::test]
async fn custom_openai_tokenizer_chunks_like_tiktoken() {
    let directory = tempfile::tempdir().unwrap();
    let settings = support::settings(4_000_000, 1, false);
    let counter = support::counter(settings, directory.path());
    let chunked = support::count_with_tokenizer(
        &counter,
        "gpt-3.5-turbo",
        "hello world",
        CustomTokenizerWire::OpenAi,
    )
    .await
    .unwrap();
    assert_eq!(chunked, 11);
}

#[tokio::test]
async fn default_encoding_initializes_once() {
    let loader = Arc::new(CountingLoader::default());
    let concrete: Arc<CountingLoader> = Arc::clone(&loader);
    let source: Arc<dyn DefaultEngine> = concrete;
    let default = DefaultEncoding::new(source);
    loader.fail_next();
    let _failure = default.get().await.unwrap_err();
    assert_eq!(loader.calls(), 1);
    let first = default.get().await.unwrap();
    assert_eq!(loader.calls(), 2);
    let second = default.get().await.unwrap();
    assert_eq!(loader.calls(), 2);
    match (&first, &second) {
        (Engine::Tiktoken(left), Engine::Tiktoken(right)) => assert!(Arc::ptr_eq(left, right)),
        _ => panic!("the default encoding is a bundled tiktoken engine"),
    }
}

#[derive(Default)]
struct CountingLoader {
    calls: AtomicUsize,
    fail: AtomicBool,
}

impl CountingLoader {
    fn fail_next(&self) {
        self.fail.store(true, Ordering::SeqCst);
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn engine() -> Engine {
        let data = assets::bundled(Encoding::Cl100kBase).unwrap();
        let engine = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &data).unwrap();
        Engine::Tiktoken(Arc::new(engine))
    }
}

#[async_trait]
impl DefaultEngine for CountingLoader {
    async fn load_default(&self) -> Result<Engine, SelectionError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        if self.fail.swap(false, Ordering::SeqCst) {
            return Err(SelectionError::Engine(EngineError::OutputTooLarge {
                ids: call,
                limit: 0,
            }));
        }
        Ok(Self::engine())
    }
}

#[tokio::test]
async fn default_encoding_is_the_bundled_cl100k() {
    let directory = tempfile::tempdir().unwrap();
    let hub = support::hub_settings("http://127.0.0.1:1", directory.path());
    let selector: Arc<dyn DefaultEngine> = Arc::new(support::selector(&hub, Settings::default()));
    let default = DefaultEncoding::new(selector);
    let engine = default.get().await.unwrap();
    assert_eq!(engine.encode("hello world").unwrap(), [15339, 1917]);
    let again = default.get().await.unwrap();
    match (&engine, &again) {
        (Engine::Tiktoken(left), Engine::Tiktoken(right)) => assert!(Arc::ptr_eq(left, right)),
        _ => panic!("the default encoding is a bundled tiktoken engine"),
    }
}
