use std::collections::BTreeSet;
use std::fs;
use std::iter;
use std::path::Path;
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
use betterlite_openai::chat_choices::wire::{StreamChoice, StreamDelta, StreamReply};
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

fn tag_split_every_byte() {
    let messages = [
        "<reasoning>alpha</reasoning>  beta",
        "<reasoning>héllo</reasoning>\n\n  grüße",
        "<reasoning>a</reasoning>b<reasoning>c</reasoning>",
        "<reasoning></reasoning>   tail",
        "no tag at all",
    ];
    for message in messages {
        let expected = split_reasoning_tag(message);
        for outcome in every_split_outcome(message) {
            assert_eq!(outcome, expected, "split of {message:?}");
        }
    }
    assert_eq!(
        split_reasoning_tag("<reasoning>a</reasoning>b<reasoning>c</reasoning>"),
        (
            Some(String::from("a")),
            String::from("b<reasoning>c</reasoning>")
        )
    );
    assert_eq!(
        split_reasoning_tag("<reasoning></reasoning>   tail"),
        (None, String::from("tail"))
    );
    assert_eq!(
        split_reasoning_tag("<reasoning>héllo</reasoning>\n\n  grüße"),
        (Some(String::from("héllo")), String::from("grüße"))
    );
}

fn reasoning_and_text() {
    let mut handler = StreamingHandler::default();
    let mut opening = reply(vec![choice(0, Some("<reasoning>th"), None)]);
    opening.choices[0].delta.role = Some(String::from("assistant"));
    handler.chunk_parser(&mut opening);
    assert_eq!(opening.choices[0].delta.content, Some(String::new()));
    assert_eq!(
        opening.choices[0].delta.reasoning_content,
        Some(String::from("th"))
    );
    assert_eq!(
        opening.choices[0].delta.role,
        Some(String::from("assistant"))
    );
    assert_eq!(opening.id, "c1");

    let mut thinking = reply(vec![choice(0, Some("ink</reasoning>\n\nanswer"), None)]);
    handler.chunk_parser(&mut thinking);
    assert_eq!(
        thinking.choices[0].delta.content,
        Some(String::from("answer"))
    );
    assert_eq!(
        thinking.choices[0].delta.reasoning_content,
        Some(String::from("ink"))
    );

    let mut finishing = reply(vec![choice(0, None, Some("stop"))]);
    handler.chunk_parser(&mut finishing);
    assert_eq!(finishing.choices[0].delta.content, None);
    assert_eq!(
        finishing.choices[0].delta.reasoning_content,
        Some(String::from("ink"))
    );

    let mut empty_finish = reply(vec![choice(0, None, Some(""))]);
    handler.chunk_parser(&mut empty_finish);
    assert_eq!(empty_finish.choices[0].delta.content, None);

    let mut appended = reply(vec![choice(0, Some("<reasoning>x</reasoning>"), None)]);
    appended.choices[0].delta.reasoning_content = Some(String::from("pre "));
    handler.chunk_parser(&mut appended);
    assert_eq!(
        appended.choices[0].delta.reasoning_content,
        Some(String::from("pre x"))
    );

    let mut duplicate = reply(vec![
        choice(1, Some("one"), None),
        choice(1, Some("two"), None),
    ]);
    handler.chunk_parser(&mut duplicate);
    assert_eq!(
        duplicate.choices[0].delta.content,
        Some(String::from("one"))
    );
    assert_eq!(
        duplicate.choices[1].delta.content,
        Some(String::from("two"))
    );

    let mut independent = StreamingHandler::default();
    let mut split = reply(vec![
        choice(0, Some("<reasoning>keep"), None),
        choice(1, Some("plain"), None),
    ]);
    independent.chunk_parser(&mut split);
    assert_eq!(split.choices[0].delta.content, Some(String::new()));
    assert_eq!(
        split.choices[0].delta.reasoning_content,
        Some(String::from("keep"))
    );
    assert_eq!(split.choices[1].delta.content, Some(String::from("plain")));
    assert_eq!(split.choices[1].delta.reasoning_content, None);
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
    let mut delta = StreamDelta::default();
    delta.content = content.map(str::to_owned);
    StreamChoice {
        index,
        finish_reason: finish.map(str::to_owned),
        delta,
        logprobs: None,
    }
}

fn unterminated_tag_flush() {
    let (state, reasoning, content) = decode_reasoning_delta(
        &ReasoningTagState::default(),
        Some("<reasoning>answer</rea"),
        false,
    );
    assert_eq!(state.phase, ReasoningPhase::Reasoning);
    assert_eq!(state.pending, "</rea");
    assert_eq!(reasoning, "answer");
    assert_eq!(content, "");
    let (drained, flushed_reasoning, flushed_content) = decode_reasoning_delta(&state, None, true);
    assert_eq!(drained, ReasoningTagState::default());
    assert_eq!(drained.phase, ReasoningPhase::Content);
    assert_eq!(flushed_reasoning, "</rea");
    assert_eq!(flushed_content, "");
    let (again, again_reasoning, again_content) = decode_reasoning_delta(&drained, None, true);
    assert_eq!(again, drained);
    assert_eq!(again_reasoning, "");
    assert_eq!(again_content, "");

    let (start_state, start_reasoning, start_content) =
        decode_reasoning_delta(&ReasoningTagState::default(), Some("<reason"), false);
    assert_eq!(start_state.phase, ReasoningPhase::Start);
    assert_eq!(start_state.pending, "<reason");
    assert_eq!(start_reasoning, "");
    assert_eq!(start_content, "");
    let (started, started_reasoning, started_content) =
        decode_reasoning_delta(&start_state, None, true);
    assert_eq!(started.phase, ReasoningPhase::Content);
    assert_eq!(started.pending, "");
    assert_eq!(started_reasoning, "");
    assert_eq!(started_content, "<reason");

    let (empty_state, empty_reasoning, empty_content) = ReasoningTagState::default().flush();
    assert_eq!(empty_state.phase, ReasoningPhase::Content);
    assert_eq!(empty_reasoning, "");
    assert_eq!(empty_content, "");
}

fn plain_text_prefix() {
    assert_eq!(ReasoningPhase::Start.name(), "start");
    assert_eq!(ReasoningPhase::Reasoning.name(), "reasoning");
    assert_eq!(ReasoningPhase::AfterClose.name(), "after_close");
    assert_eq!(ReasoningPhase::Content.name(), "content");
    let initial = ReasoningTagState::default();
    assert_eq!(initial.phase, ReasoningPhase::Start);
    assert_eq!(initial.pending, "");

    let (after_lt, lt_reasoning, lt_content) = initial.feed("<");
    assert_eq!(after_lt.phase, ReasoningPhase::Start);
    assert_eq!(after_lt.pending, "<");
    assert_eq!(lt_reasoning, "");
    assert_eq!(lt_content, "");
    let (after_empty, empty_reasoning, empty_content) = after_lt.feed("");
    assert_eq!(after_empty.phase, ReasoningPhase::Start);
    assert_eq!(after_empty.pending, "<");
    assert_eq!(empty_reasoning, "");
    assert_eq!(empty_content, "");
    let (mismatched, mismatch_reasoning, mismatch_content) = after_empty.feed("<rea");
    assert_eq!(mismatched.phase, ReasoningPhase::Content);
    assert_eq!(mismatched.pending, "");
    assert_eq!(mismatch_reasoning, "");
    assert_eq!(mismatch_content, "<<rea");

    let (opened, opened_reasoning, opened_content) =
        ReasoningTagState::default().feed("<reasoning>seen");
    assert_eq!(opened.phase, ReasoningPhase::Reasoning);
    assert_eq!(opened.pending, "");
    assert_eq!(opened_reasoning, "seen");
    assert_eq!(opened_content, "");

    let (whitespace, whitespace_reasoning, whitespace_content) =
        ReasoningTagState::default().feed(" <reasoning>x");
    assert_eq!(whitespace.phase, ReasoningPhase::Content);
    assert_eq!(whitespace_reasoning, "");
    assert_eq!(whitespace_content, " <reasoning>x");

    let (think, think_reasoning, think_content) = ReasoningTagState::default().feed("<think>x");
    assert_eq!(think.phase, ReasoningPhase::Content);
    assert_eq!(think_reasoning, "");
    assert_eq!(think_content, "<think>x");

    let (leading, leading_reasoning, leading_content) =
        ReasoningTagState::default().feed("<reasoninX>x");
    assert_eq!(leading.phase, ReasoningPhase::Content);
    assert_eq!(leading_reasoning, "");
    assert_eq!(leading_content, "<reasoninX>x");
}

fn params(source: &str) -> ParamMap {
    serde_json::from_str::<ParamMap>(source).unwrap()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn non_string_effort() {
    assert_eq!(non_string_reasoning_effort(&params("{}")), BTreeSet::new());
    assert_eq!(
        non_string_reasoning_effort(&params(r#"{"reasoning_effort":null}"#)),
        BTreeSet::new()
    );
    assert_eq!(
        non_string_reasoning_effort(&params(r#"{"reasoning_effort":"high"}"#)),
        BTreeSet::new()
    );
    for source in [
        r#"{"reasoning_effort":1}"#,
        r#"{"reasoning_effort":true}"#,
        r#"{"reasoning_effort":[1]}"#,
        r#"{"reasoning_effort":{"a":1}}"#,
    ] {
        assert_eq!(
            non_string_reasoning_effort(&params(source)),
            set(&["reasoning_effort"])
        );
    }

    let gpt = "openai.gpt-6-sol";
    let refused = set(&[
        "temperature",
        "top_p",
        "frequency_penalty",
        "presence_penalty",
        "logprobs",
        "top_logprobs",
    ]);
    assert_eq!(
        params_refused_while_reasoning(gpt, &params("{}")).unwrap(),
        BTreeSet::new()
    );
    assert_eq!(
        params_refused_while_reasoning(gpt, &params(r#"{"temperature":0.2}"#)).unwrap(),
        set(&["temperature"])
    );
    assert_eq!(
        params_refused_while_reasoning(
            gpt,
            &params(r#"{"temperature":null,"top_p":null,"other":1}"#)
        )
        .unwrap(),
        set(&["temperature", "top_p"])
    );
    assert_eq!(
        params_refused_while_reasoning(
            gpt,
            &params(
                r#"{"temperature":1,"top_p":1,"frequency_penalty":1,"presence_penalty":1,"logprobs":1,"top_logprobs":1}"#
            )
        )
        .unwrap(),
        refused
    );
    assert_eq!(
        params_refused_while_reasoning(
            gpt,
            &params(r#"{"reasoning_effort":"none","temperature":1}"#)
        )
        .unwrap(),
        BTreeSet::new()
    );
    assert_eq!(
        params_refused_while_reasoning(
            "anthropic.claude-3-5-sonnet",
            &params(r#"{"temperature":1,"reasoning_effort":"high"}"#)
        )
        .unwrap(),
        BTreeSet::new()
    );
    assert_eq!(
        params_refused_while_reasoning("openai.gpt-oss-20b-1:0", &params(r#"{"temperature":1}"#))
            .unwrap(),
        BTreeSet::new()
    );
    assert_eq!(
        params_refused_while_reasoning(gpt, &params(r#"{"reasoning_effort":1,"temperature":1}"#))
            .unwrap(),
        set(&["temperature"])
    );
}

fn model_refused_fields() {
    assert_eq!(
        params_refused_for("openai.gpt-oss-20b-1:0"),
        set(&["logit_bias"])
    );
    assert_eq!(
        params_refused_for("us-east-1/openai.gpt-oss-20b-1:0"),
        set(&["logit_bias"])
    );
    assert_eq!(
        params_refused_for("xai.grok-4"),
        set(&["frequency_penalty", "presence_penalty"])
    );
    assert_eq!(
        params_refused_for("us-east-1/xai.grok-4"),
        set(&["frequency_penalty", "presence_penalty"])
    );
    assert_eq!(
        params_refused_for("openai.gpt-oss-xai.grok"),
        set(&["frequency_penalty", "logit_bias", "presence_penalty"])
    );
    assert_eq!(params_refused_for("OpenAI.GPT-OSS-20b"), BTreeSet::new());
    assert_eq!(
        params_refused_for("anthropic.claude-3-5-sonnet"),
        BTreeSet::new()
    );

    assert_eq!(reasoning_efforts_refused_for("xai.grok-4"), set(&["none"]));
    assert_eq!(
        reasoning_efforts_refused_for("us-east-1/xai.grok-4"),
        set(&["none"])
    );
    assert_eq!(
        reasoning_efforts_refused_for("openai.gpt-oss-20b-1:0"),
        BTreeSet::new()
    );

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
        without_refused_effort("xai.grok-4", &params(r#"{"reasoning_effort":"high"}"#)),
        params(r#"{"reasoning_effort":"high"}"#)
    );
    assert_eq!(
        without_refused_effort("openai.gpt-oss-20b-1:0", &source),
        source
    );
    let non_string = params(r#"{"reasoning_effort":1}"#);
    assert_eq!(
        without_refused_effort("xai.grok-4", &non_string),
        non_string
    );
}

const INIT: &str = r#"{"model":"m","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const THINKING: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"openai.gpt-oss-20b-1:0","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"<reasoning>thin</reasoning>\n\nbody","reasoning_content":"pre "}},{"index":1,"finish_reason":"stop","message":{"role":"assistant","content":"<reasoning></reasoning>  gone"}},{"index":2,"finish_reason":"stop","message":{"role":"assistant","content":null}}],"usage":{"prompt_tokens":3,"completion_tokens":4,"total_tokens":7}}"#;

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

fn split_reply(
    bytes: &[u8],
    headers: &IndexMap<String, String>,
    existing: &AdditionalHeaders,
) -> RuntimeReply {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("trace"),
        Duration::default,
    )
    .unwrap();
    let sink = Sink;
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &redactor,
        log: &sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").unwrap(),
    };
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
    transform_response(
        bytes,
        &mut state,
        &env,
        input,
        &ReplyInput {
            options,
            provider_headers: headers,
            existing_additional_headers: existing,
        },
    )
    .unwrap()
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
    assert_eq!(reply.hidden_model, None);
    assert_eq!(reply.response.id, "chatcmpl-1");
    assert_eq!(reply.response.created, 1_700_000_000);
    assert_eq!(reply.response.model, "openai.gpt-oss-20b-1:0");
    assert_eq!(
        reply.response.choices[0].message.content,
        Some(String::from("body"))
    );
    assert_eq!(
        reply.response.choices[1].message.content,
        Some(String::from("gone"))
    );
    assert_eq!(reply.response.choices[2].message.content, None);
    assert_eq!(reply.choices[0].content, Some(String::from("body")));
    assert_eq!(
        reply.choices[0].reasoning_content,
        Some(String::from("pre thin"))
    );
    assert_eq!(reply.choices[0].finish_reason, Some(String::from("stop")));
    assert_eq!(reply.choices[0].index, 0);
    assert_eq!(reply.choices[1].content, Some(String::from("gone")));
    assert_eq!(reply.choices[1].reasoning_content, None);
    assert_eq!(reply.choices[2].content, None);
    assert_eq!(reply.choices[2].reasoning_content, None);
    assert_eq!(reply.headers.headers, headers);
    assert_eq!(
        reply
            .headers
            .additional_headers
            .get("llm_provider-x-request-id"),
        Some(&Json::Text(String::from("existing")))
    );
    assert_eq!(
        reply
            .headers
            .additional_headers
            .get("llm_provider-x-ratelimit-remaining-requests"),
        Some(&Json::Text(String::from("9")))
    );
    assert_eq!(
        reply
            .headers
            .additional_headers
            .get("x-ratelimit-remaining-requests"),
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
            take = at + character.len_utf8();
            if count + 1 == width {
                break;
            }
        }
        let (front, back) = rest.split_at(take);
        let (next, fed_reasoning, fed_content) = decode_reasoning_delta(&state, Some(front), false);
        state = next;
        reasoning.push_str(&fed_reasoning);
        content.push_str(&fed_content);
        rest = back;
        width = if width == 3 { 1 } else { width + 1 };
    }
    let (_, flushed_reasoning, flushed_content) = decode_reasoning_delta(&state, None, true);
    reasoning.push_str(&flushed_reasoning);
    content.push_str(&flushed_content);
    (reasoning, content)
}

fn category(bytes: &[u8]) -> &'static str {
    let Ok(text) = std::str::from_utf8(bytes) else {
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
    let mut seen = 0;
    let entries = fs::read_dir(corpus).unwrap();
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            continue;
        }
        let bytes = fs::read(&path).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        let expected = name.rsplit('-').next().unwrap();
        assert_eq!(category(&bytes), expected, "seed {name}");
        seen += 1;
    }
    assert!(seen > 0, "no seeds under {}", corpus.display());
}

#[test]
fn held_prefix_keeps_the_longest_close_fragment() {
    assert_eq!(held_close_tag_start("abc"), 3);
    assert_eq!(held_close_tag_start("abc</rea"), 3);
    assert_eq!(held_close_tag_start("abc</"), 3);
    assert_eq!(held_close_tag_start("abc<"), 3);
    assert_eq!(held_close_tag_start("héllo</reasoni"), 6);
    assert_eq!(held_close_tag_start("</reasoning>"), "</reasoning>".len());
}
