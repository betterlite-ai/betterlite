#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::iter;
use std::path::Path;
use std::str::from_utf8;
use std::time::Duration;

use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use betterlite_openai::chat_choices::stream::CHUNK_OBJECT;
use betterlite_openai::chat_choices::wire::{
    ProviderField, StreamChoice, StreamDelta, StreamReply,
};
use betterlite_openai::compatible_wire::{LogInput, NEW_ID, ReplyOptions, system_clock};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;

use super::buffered::{ReplyInput, RuntimeReply, transform_response};
use super::refusals::{
    non_string_reasoning_effort, params_refused_for, params_refused_while_reasoning,
    reasoning_efforts_refused_for, without_params, without_refused_effort,
};
use super::stream::StreamingHandler;
use super::{
    ReasoningPhase, ReasoningTagState, decode_reasoning_delta, held_close_tag_start,
    split_reasoning_tag,
};
use crate::model_routes::ParamMap;

#[rstest]
#[case::tag_split_every_byte(0_u8)]
#[case::reasoning_and_text(1_u8)]
#[case::unterminated_tag_flush(2_u8)]
#[case::plain_text_prefix(3_u8)]
#[case::non_string_effort(4_u8)]
#[case::model_refused_fields(5_u8)]
#[case::buffered_thinking_split(6_u8)]
fn reasoning_stream_cases(#[case] case: u8) {
    match case {
        0 => tag_split_every_byte(),
        1 => reasoning_and_text(),
        2 => unterminated_tag_flush(),
        3 => plain_text_prefix(),
        4 => non_string_effort(),
        5 => model_refused_fields(),
        6 => buffered_thinking_split(),
        _ => panic!("case {case} has no body"),
    }
}

fn every_split_outcome(message: &str) -> Vec<(Option<String>, String)> {
    let mut outcomes = Vec::new();
    for (at, _character) in message
        .char_indices()
        .chain(iter::once((message.len(), ' ')))
    {
        assert!(message.is_char_boundary(at));
        let (front, back) = message.split_at(at);
        let (state, mut reasoning, mut content) =
            decode_reasoning_delta(&ReasoningTagState::default(), Some(front), false);
        let (drained, later_reasoning, later_content) =
            decode_reasoning_delta(&state, Some(back), true);
        reasoning.push_str(&later_reasoning);
        content.push_str(&later_content);
        assert_eq!(drained.phase, ReasoningPhase::Content);
        let answer = (!reasoning.is_empty()).then_some(reasoning);
        outcomes.push((answer, content));
    }
    outcomes
}

const SPLIT_MESSAGES: [(&str, Option<&str>, &str); 6] = [
    ("<reasoning>alpha</reasoning>  beta", Some("alpha"), "beta"),
    (
        "<reasoning>h\u{e9}llo</reasoning>\n\n  gr\u{fc}\u{df}e",
        Some("h\u{e9}llo"),
        "gr\u{fc}\u{df}e",
    ),
    (
        "<reasoning>a</reasoning>b<reasoning>c</reasoning>",
        Some("a"),
        "b<reasoning>c</reasoning>",
    ),
    ("<reasoning></reasoning>   tail", None, "tail"),
    (
        "<reasoning>a</reasoning>\u{1c}\u{1f}\u{a0} b",
        Some("a"),
        "b",
    ),
    ("no tag at all", None, "no tag at all"),
];

fn tag_split_every_byte() {
    for (message, reasoning, body) in SPLIT_MESSAGES {
        let expected = (reasoning.map(String::from), String::from(body));
        assert_eq!(split_reasoning_tag(message), expected);
        for outcome in every_split_outcome(message) {
            assert_eq!(outcome, expected, "split of {message:?}");
        }
    }
}

fn reasoning_and_text() {
    let mut handler = StreamingHandler::default();
    stream_opening_and_thinking(&mut handler);
    stream_finish(&mut handler);
    stream_flush_at_finish();
    stream_appended_reasoning();
    stream_duplicate_index();
    stream_independent_choices();
}

fn stream_opening_and_thinking(handler: &mut StreamingHandler) {
    let mut opening = reply(vec![choice(0, Some("<reasoning>th"), None)]);
    opening.choices[0].delta.role = Some(String::from("assistant"));
    handler.chunk_parser(&mut opening);
    let opening_delta = &opening.choices[0].delta;
    assert_eq!(
        (
            opening_delta.content.as_deref(),
            opening_delta.reasoning_content.as_deref(),
            opening_delta.role.as_deref()
        ),
        (Some(""), Some("th"), Some("assistant"))
    );
    assert_eq!(opening.id, "c1");

    let mut thinking = reply(vec![choice(0, Some("ink</reasoning>\n\nanswer"), None)]);
    handler.chunk_parser(&mut thinking);
    let thinking_delta = &thinking.choices[0].delta;
    assert_eq!(
        (
            thinking_delta.content.as_deref(),
            thinking_delta.reasoning_content.as_deref()
        ),
        (Some("answer"), Some("ink"))
    );
}

fn stream_finish(handler: &mut StreamingHandler) {
    let mut finishing = reply(vec![choice(0, None, Some("stop"))]);
    handler.chunk_parser(&mut finishing);
    let finishing_delta = &finishing.choices[0].delta;
    assert_eq!(
        (
            finishing_delta.content.as_deref(),
            finishing_delta.reasoning_content.as_deref()
        ),
        (None, None)
    );

    let mut empty_finish = reply(vec![choice(0, None, Some(""))]);
    handler.chunk_parser(&mut empty_finish);
    assert_eq!(empty_finish.choices[0].delta.content, None);
}

fn stream_flush_at_finish() {
    let mut handler = StreamingHandler::default();
    let mut held = reply(vec![choice(0, Some("<reasoning>x</re"), None)]);
    handler.chunk_parser(&mut held);
    let held_delta = &held.choices[0].delta;
    assert_eq!(
        (
            held_delta.content.as_deref(),
            held_delta.reasoning_content.as_deref()
        ),
        (Some(""), Some("x"))
    );

    let mut finishing = reply(vec![choice(0, None, Some("stop"))]);
    handler.chunk_parser(&mut finishing);
    let finishing_delta = &finishing.choices[0].delta;
    assert_eq!(
        (
            finishing_delta.content.as_deref(),
            finishing_delta.reasoning_content.as_deref()
        ),
        (None, Some("</re"))
    );
}

fn stream_appended_reasoning() {
    let mut handler = StreamingHandler::default();
    let mut appended = reply(vec![choice(0, Some("<reasoning>x</reasoning>"), None)]);
    appended.choices[0].delta.reasoning_content = Some(String::from("pre "));
    handler.chunk_parser(&mut appended);
    let appended_delta = &appended.choices[0].delta;
    assert_eq!(
        (
            appended_delta.content.as_deref(),
            appended_delta.reasoning_content.as_deref()
        ),
        (Some(""), Some("pre x"))
    );
}

fn stream_duplicate_index() {
    let mut handler = StreamingHandler::default();
    let mut duplicate = reply(vec![
        choice(1, Some("one"), None),
        choice(1, Some("two"), None),
    ]);
    handler.chunk_parser(&mut duplicate);
    assert_eq!(
        (
            duplicate.choices[0].delta.content.as_deref(),
            duplicate.choices[1].delta.content.as_deref()
        ),
        (Some("one"), Some("two"))
    );
}

fn stream_independent_choices() {
    let mut handler = StreamingHandler::default();
    let mut split = reply(vec![
        choice(0, Some("<reasoning>keep"), None),
        choice(1, Some("plain"), None),
    ]);
    handler.chunk_parser(&mut split);
    let reasoning_delta = &split.choices[0].delta;
    let plain_delta = &split.choices[1].delta;
    assert_eq!(
        (
            reasoning_delta.content.as_deref(),
            reasoning_delta.reasoning_content.as_deref()
        ),
        (Some(""), Some("keep"))
    );
    assert_eq!(
        (
            plain_delta.content.as_deref(),
            plain_delta.reasoning_content.as_deref()
        ),
        (Some("plain"), None)
    );
}

fn reply(choices: Vec<StreamChoice>) -> StreamReply {
    StreamReply {
        id: String::from("c1"),
        object: CHUNK_OBJECT,
        created: 10,
        model: Some(String::from("openai.gpt-oss-20b-1:0")),
        choices,
        usage: None,
        service_tier: Some(String::from("default")),
    }
}

fn choice(index: i64, content: Option<&str>, finish: Option<&str>) -> StreamChoice {
    StreamChoice {
        index,
        finish_reason: finish.map(str::to_owned),
        delta: StreamDelta {
            role: None,
            content: content.map(str::to_owned),
            function_call: None,
            tool_calls: None,
            audio: None,
            reasoning_content: None,
            reasoning: ProviderField::Absent,
            passthrough: StreamDelta::default().passthrough,
        },
        logprobs: None,
    }
}

fn unterminated_tag_flush() {
    let (state, reasoning, content) = decode_reasoning_delta(
        &ReasoningTagState::default(),
        Some("<reasoning>answer</rea"),
        false,
    );
    assert_eq!(
        (
            state.phase,
            state.pending.as_str(),
            reasoning.as_str(),
            content.as_str()
        ),
        (ReasoningPhase::Reasoning, "</rea", "answer", "")
    );
    let (drained, flushed_reasoning, flushed_content) = decode_reasoning_delta(&state, None, true);
    assert_eq!(
        (
            drained.phase,
            drained.pending.as_str(),
            flushed_reasoning.as_str(),
            flushed_content.as_str()
        ),
        (ReasoningPhase::Content, "", "</rea", "")
    );
    let (again, again_reasoning, again_content) = decode_reasoning_delta(&drained, None, true);
    assert_eq!(again, drained);
    assert_eq!((again_reasoning.as_str(), again_content.as_str()), ("", ""));
    unterminated_open_flush();
}

fn unterminated_open_flush() {
    let (start_state, start_reasoning, start_content) =
        decode_reasoning_delta(&ReasoningTagState::default(), Some("<reason"), false);
    assert_eq!(
        (
            start_state.phase,
            start_state.pending.as_str(),
            start_reasoning.as_str(),
            start_content.as_str()
        ),
        (ReasoningPhase::Start, "<reason", "", "")
    );
    let (started, started_reasoning, started_content) =
        decode_reasoning_delta(&start_state, None, true);
    assert_eq!(
        (
            started.phase,
            started.pending.as_str(),
            started_reasoning.as_str(),
            started_content.as_str()
        ),
        (ReasoningPhase::Content, "", "", "<reason")
    );
    empty_flush();
}

fn empty_flush() {
    let (empty_state, empty_reasoning, empty_content) = ReasoningTagState::default().flush();
    assert_eq!(
        (
            empty_state.phase,
            empty_reasoning.as_str(),
            empty_content.as_str()
        ),
        (ReasoningPhase::Content, "", "")
    );
}

fn plain_text_prefix() {
    assert_eq!(ReasoningPhase::Start.name(), "start");
    assert_eq!(ReasoningPhase::Reasoning.name(), "reasoning");
    assert_eq!(ReasoningPhase::AfterClose.name(), "after_close");
    assert_eq!(ReasoningPhase::Content.name(), "content");
    let initial = ReasoningTagState::default();
    assert_eq!(
        (initial.phase, initial.pending.as_str()),
        (ReasoningPhase::Start, "")
    );
    partial_open_tag_feeds();
    non_matching_prefix_feeds();
}

fn partial_open_tag_feeds() {
    let (after_lt, lt_reasoning, lt_content) = ReasoningTagState::default().feed("<");
    assert_eq!(
        (
            after_lt.phase,
            after_lt.pending.as_str(),
            lt_reasoning.as_str(),
            lt_content.as_str()
        ),
        (ReasoningPhase::Start, "<", "", "")
    );
    let (after_empty, empty_reasoning, empty_content) = after_lt.feed("");
    assert_eq!(
        (
            after_empty.phase,
            after_empty.pending.as_str(),
            empty_reasoning.as_str(),
            empty_content.as_str()
        ),
        (ReasoningPhase::Start, "<", "", "")
    );
    let (mismatched, mismatch_reasoning, mismatch_content) = after_empty.feed("<rea");
    assert_eq!(
        (
            mismatched.phase,
            mismatched.pending.as_str(),
            mismatch_reasoning.as_str(),
            mismatch_content.as_str()
        ),
        (ReasoningPhase::Content, "", "", "<<rea")
    );
}

fn non_matching_prefix_feeds() {
    let cases = [
        ("<reasoning>seen", ReasoningPhase::Reasoning, "seen", ""),
        (
            " <reasoning>x",
            ReasoningPhase::Content,
            "",
            " <reasoning>x",
        ),
        ("<think>x", ReasoningPhase::Content, "", "<think>x"),
        ("<reasoninX>x", ReasoningPhase::Content, "", "<reasoninX>x"),
    ];
    for (text, phase, reasoning, content) in cases {
        let (state, fed_reasoning, fed_content) = ReasoningTagState::default().feed(text);
        assert_eq!(
            (
                state.phase,
                state.pending.as_str(),
                fed_reasoning.as_str(),
                fed_content.as_str()
            ),
            (phase, "", reasoning, content),
            "feed of {text:?}"
        );
    }
}

fn params(source: &str) -> ParamMap {
    serde_json::from_str::<ParamMap>(source).unwrap()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

const GPT: &str = "openai.gpt-6-sol";
const SIX_REFUSED: &str = r#"{"temperature":1,"top_p":1,"frequency_penalty":1,"presence_penalty":1,"logprobs":1,"top_logprobs":1}"#;

fn non_string_effort() {
    let cases: [(&str, &[&str]); 7] = [
        ("{}", &[]),
        (r#"{"reasoning_effort":null}"#, &[]),
        (r#"{"reasoning_effort":"high"}"#, &[]),
        (r#"{"reasoning_effort":1}"#, &["reasoning_effort"]),
        (r#"{"reasoning_effort":true}"#, &["reasoning_effort"]),
        (r#"{"reasoning_effort":[1]}"#, &["reasoning_effort"]),
        (r#"{"reasoning_effort":{"a":1}}"#, &["reasoning_effort"]),
    ];
    for (source, expected) in cases {
        assert_eq!(
            non_string_reasoning_effort(&params(source)),
            set(expected),
            "params {source}"
        );
    }
    refused_while_reasoning_on_gpt();
    refused_while_reasoning_elsewhere();
}

fn refused_while_reasoning_on_gpt() {
    let cases: [(&str, &[&str]); 6] = [
        ("{}", &[]),
        (r#"{"temperature":0.2}"#, &["temperature"]),
        (
            r#"{"temperature":null,"top_p":null,"other":1}"#,
            &["temperature", "top_p"],
        ),
        (
            SIX_REFUSED,
            &[
                "temperature",
                "top_p",
                "frequency_penalty",
                "presence_penalty",
                "logprobs",
                "top_logprobs",
            ],
        ),
        (r#"{"reasoning_effort":"none","temperature":1}"#, &[]),
        (
            r#"{"reasoning_effort":1,"temperature":1}"#,
            &["temperature"],
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(
            params_refused_while_reasoning(GPT, &params(source)).unwrap(),
            set(expected),
            "params {source}"
        );
    }
}

fn refused_while_reasoning_elsewhere() {
    for model in ["anthropic.claude-3-5-sonnet", "openai.gpt-oss-20b-1:0"] {
        assert_eq!(
            params_refused_while_reasoning(model, &params(SIX_REFUSED)).unwrap(),
            BTreeSet::new(),
            "model {model}"
        );
    }
}

fn model_refused_fields() {
    refused_fields_by_model();
    refused_efforts_by_model();
    dropped_params();
}

fn refused_fields_by_model() {
    let fields: [(&str, &[&str]); 7] = [
        ("openai.gpt-oss-20b-1:0", &["logit_bias"]),
        ("us-east-1/openai.gpt-oss-20b-1:0", &["logit_bias"]),
        ("xai.grok-4", &["frequency_penalty", "presence_penalty"]),
        (
            "us-east-1/xai.grok-4",
            &["frequency_penalty", "presence_penalty"],
        ),
        (
            "openai.gpt-oss-xai.grok",
            &["frequency_penalty", "logit_bias", "presence_penalty"],
        ),
        ("OpenAI.GPT-OSS-20b", &[]),
        ("anthropic.claude-3-5-sonnet", &[]),
    ];
    for (model, expected) in fields {
        assert_eq!(params_refused_for(model), set(expected), "model {model}");
    }
}

fn refused_efforts_by_model() {
    let efforts: [(&str, &[&str]); 3] = [
        ("xai.grok-4", &["none"]),
        ("us-east-1/xai.grok-4", &["none"]),
        ("openai.gpt-oss-20b-1:0", &[]),
    ];
    for (model, expected) in efforts {
        assert_eq!(
            reasoning_efforts_refused_for(model),
            set(expected),
            "model {model}"
        );
    }
}

fn dropped_params() {
    let source = params(r#"{"logit_bias":{"a":1},"temperature":0.2,"reasoning_effort":"none"}"#);
    let dropped = set(&["logit_bias"]);
    let without = without_params(&source, &dropped);
    assert_eq!(without.len(), 2);
    assert!(!without.contains_key("logit_bias"));
    assert_eq!(without.get("temperature"), source.get("temperature"));
    assert_eq!(
        without.get("reasoning_effort"),
        source.get("reasoning_effort")
    );

    let removed = without_refused_effort("xai.grok-4", &source);
    assert_eq!(removed.len(), 2);
    assert!(!removed.contains_key("reasoning_effort"));
    assert_eq!(
        without_refused_effort("openai.gpt-oss-20b-1:0", &source),
        source
    );
    for text in [
        r#"{"reasoning_effort":"high"}"#,
        r#"{"reasoning_effort":1}"#,
    ] {
        let unchanged = params(text);
        assert_eq!(without_refused_effort("xai.grok-4", &unchanged), unchanged);
    }
}

const INIT: &str = r#"{"model":"m","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const THINKING: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"openai.gpt-oss-20b-1:0","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"<reasoning>thin</reasoning>\n\nbody","reasoning_content":"pre "}},{"index":1,"finish_reason":"stop","message":{"role":"assistant","content":"<reasoning></reasoning>  gone"}},{"index":2,"finish_reason":"stop","message":{"role":"assistant","content":null}}],"usage":{"prompt_tokens":3,"completion_tokens":4,"total_tokens":7}}"#;

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

fn logging_state(redactor: &Redactor) -> LoggingState {
    LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .unwrap()
}

fn pre_post_env<'env>(redactor: &'env Redactor, sink: &'env Sink) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").unwrap(),
    }
}

fn split_reply(
    bytes: &[u8],
    headers: &IndexMap<String, String>,
    existing: &AdditionalHeaders,
) -> RuntimeReply {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let sink = Sink;
    let env = pre_post_env(&redactor, &sink);
    let options = ReplyOptions {
        json_mode: false,
        provider: None,
        base_model: None,
        identifier: NEW_ID,
        clock: system_clock,
    };
    let input = LogInput {
        messages: Json::List(Vec::new()),
        data: RawBody::default(),
    };
    let reply_input = ReplyInput {
        options,
        provider_headers: headers,
        existing_additional_headers: existing,
    };
    transform_response(bytes, &mut state, &env, input, &reply_input).unwrap()
}

fn buffered_thinking_split() {
    let headers: IndexMap<String, String> = IndexMap::from([
        (String::from("x-request-id"), String::from("r1")),
        (
            String::from("x-ratelimit-remaining-requests"),
            String::from("9"),
        ),
    ]);
    let existing: AdditionalHeaders = AdditionalHeaders::from([(
        String::from("llm_provider-x-request-id"),
        Json::Text(String::from("existing")),
    )]);
    let reply = split_reply(THINKING.as_bytes(), &headers, &existing);
    buffered_response(&reply);
    buffered_choices(&reply);
    buffered_headers(&reply, &headers);
}

fn buffered_response(reply: &RuntimeReply) {
    assert_eq!(reply.hidden_model, None);
    assert_eq!(reply.response.id, "chatcmpl-1");
    assert_eq!(reply.response.created, 1_700_000_000);
    assert_eq!(reply.response.model, "openai.gpt-oss-20b-1:0");
    assert_eq!(
        (
            reply.response.choices[0].message.content.as_deref(),
            reply.response.choices[1].message.content.as_deref(),
            reply.response.choices[2].message.content.as_deref()
        ),
        (Some("body"), Some("gone"), None)
    );
}

fn buffered_choices(reply: &RuntimeReply) {
    let first = &reply.choices[0];
    assert_eq!(
        (
            first.content.as_deref(),
            first.reasoning_content.as_deref(),
            first.finish_reason.as_deref(),
            first.index
        ),
        (Some("body"), Some("pre thin"), Some("stop"), 0)
    );
    let second = &reply.choices[1];
    assert_eq!(
        (
            second.content.as_deref(),
            second.reasoning_content.as_deref()
        ),
        (Some("gone"), None)
    );
    let third = &reply.choices[2];
    assert_eq!(
        (third.content.as_deref(), third.reasoning_content.as_deref()),
        (None, None)
    );
}

fn buffered_headers(reply: &RuntimeReply, headers: &IndexMap<String, String>) {
    let additional = &reply.headers.additional_headers;
    assert_eq!(&reply.headers.headers, headers);
    assert_eq!(
        additional.get("llm_provider-x-request-id"),
        Some(&Json::Text(String::from("existing")))
    );
    assert_eq!(
        additional.get("llm_provider-x-ratelimit-remaining-requests"),
        Some(&Json::Text(String::from("9")))
    );
    assert_eq!(
        additional.get("x-ratelimit-remaining-requests"),
        Some(&Json::Text(String::from("9")))
    );
}

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fuzz/corpus/bedrock_reasoning_stream"
);

fn replay(text: &str) -> (String, String) {
    let mut state = ReasoningTagState::default();
    let mut reasoning = String::new();
    let mut content = String::new();
    let mut rest = text;
    let mut width = 1_usize;
    while !rest.is_empty() {
        let mut take = 0;
        for (count, (at, character)) in rest.char_indices().enumerate() {
            take = at.saturating_add(character.len_utf8());
            if count.saturating_add(1) == width {
                break;
            }
        }
        let (front, back) = rest.split_at(take);
        let (next, fed_reasoning, fed_content) = decode_reasoning_delta(&state, Some(front), false);
        state = next;
        reasoning.push_str(&fed_reasoning);
        content.push_str(&fed_content);
        rest = back;
        width = if width == 3 {
            1
        } else {
            width.saturating_add(1)
        };
    }
    let (_, flushed_reasoning, flushed_content) = decode_reasoning_delta(&state, None, true);
    reasoning.push_str(&flushed_reasoning);
    content.push_str(&flushed_content);
    (reasoning, content)
}

fn category(bytes: &[u8]) -> &'static str {
    let Ok(text) = from_utf8(bytes) else {
        return "invalid";
    };
    let (reasoning, content) = replay(text);
    match (!reasoning.is_empty(), !content.is_empty()) {
        (true, true) => "both",
        (true, false) => "reasoning",
        (false, true) => "content",
        (false, false) => "empty",
    }
}

#[test]
fn replays_the_bedrock_reasoning_stream_corpus() {
    let corpus = Path::new(CORPUS);
    let mut seen = 0_usize;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            if path.is_dir() {
                return Ok(());
            }
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap();
            let expected = name.rsplit('-').next().unwrap();
            assert_eq!(category(&bytes), expected, "seed {name}");
            seen = seen.saturating_add(1);
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(seen > 0, "no seeds under {}", corpus.display());
}

#[test]
fn held_prefix_keeps_the_longest_close_fragment() {
    assert_eq!(held_close_tag_start("abc"), 3);
    assert_eq!(held_close_tag_start("abc</rea"), 3);
    assert_eq!(held_close_tag_start("abc</"), 3);
    assert_eq!(held_close_tag_start("abc<"), 3);
    assert_eq!(held_close_tag_start("h\u{e9}llo</reasoni"), 6);
    assert_eq!(held_close_tag_start("</reasoning>"), "</reasoning>".len());
}
