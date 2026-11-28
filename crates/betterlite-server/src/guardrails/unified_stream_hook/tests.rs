#![cfg(test)]

mod support;

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_core::routes::CallType;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::unified_dispatch::registry::TranslationEntry;
use indexmap::IndexMap;
use rstest::rstest;

use super::StreamFlagSources;
use super::settings::{InvalidFlag, StreamSettings, TransformMode};
use crate::guardrails::anthropic_translation::block_continuation;
use crate::guardrails::unified_stream::StreamOutput;
use support::{
    Harness, KeyMode, NAME, ScanCall, SourceFailure, capture_logs, chunk, content, fake, lines,
    modify_response, settings, texts,
};

const MASK_WARNING: &str = "UnifiedLLMGuardrails: streaming_buffer_until_moderated is disabled for stream-guard because mask_response_content=True -- buffered replay would release unredacted original chunks instead of the moderated output.";
const FALLBACK_WARNING: &str = "UnifiedLLMGuardrails: streaming_transform_mode=incremental_diff is only supported for the OpenAI chat completions streaming path with a resolvable request route; falling back to block_only for stream-guard";

fn base(mask_response_content: bool) -> GuardrailBase {
    let mut guardrail_settings = settings();
    guardrail_settings.mask_response_content = mask_response_content;
    GuardrailBase {
        settings: guardrail_settings,
        mode: None,
    }
}

fn flags(entries: &[(&str, GuardrailValue)]) -> BTreeMap<String, GuardrailValue> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn scans(items: &[usize]) -> Vec<ScanCall> {
    items
        .iter()
        .map(|count| ScanCall {
            items: *count,
            sink: false,
        })
        .collect()
}

fn block() -> Result<(), GuardrailFailure> {
    Err(modify_response("blocked", None))
}

fn http(status: u16) -> Result<(), GuardrailFailure> {
    Err(GuardrailFailure::Http {
        status,
        detail: String::from("bad"),
    })
}

#[rstest]
#[case::live(false, false)]
#[case::buffered(true, true)]
fn settings_default_every_flag(#[case] buffer_default: bool, #[case] end_of_stream_only: bool) {
    let resolved =
        StreamSettings::resolve(&StreamFlagSources::default(), buffer_default, &base(false));
    let expected = StreamSettings {
        sampling_rate: 5,
        end_of_stream_only,
        transform_mode: TransformMode::BlockOnly,
        buffer_until_moderated: buffer_default,
    };
    assert_eq!(resolved, Ok(expected));
}

#[rstest]
#[case::attribute(&[("streaming_sampling_rate", GuardrailValue::SignedInteger(2))], &[], &[], 2)]
#[case::config_over_attribute(&[("streaming_sampling_rate", GuardrailValue::SignedInteger(2))], &[("streaming_sampling_rate", GuardrailValue::SignedInteger(3))], &[], 3)]
#[case::optional_over_config(&[("streaming_sampling_rate", GuardrailValue::SignedInteger(2))], &[("streaming_sampling_rate", GuardrailValue::SignedInteger(3))], &[("streaming_sampling_rate", GuardrailValue::SignedInteger(4))], 4)]
#[case::optional_over_attribute(&[("streaming_sampling_rate", GuardrailValue::SignedInteger(2))], &[], &[("streaming_sampling_rate", GuardrailValue::SignedInteger(4))], 4)]
#[case::rate_of_one(&[], &[], &[("streaming_sampling_rate", GuardrailValue::SignedInteger(1))], 1)]
#[case::unsigned(&[], &[("streaming_sampling_rate", GuardrailValue::UnsignedInteger(7))], &[], 7)]
#[case::null_takes_the_default(&[("streaming_sampling_rate", GuardrailValue::SignedInteger(2))], &[], &[("streaming_sampling_rate", GuardrailValue::Null)], 5)]
fn settings_later_source_wins(
    #[case] attributes: &[(&str, GuardrailValue)],
    #[case] config: &[(&str, GuardrailValue)],
    #[case] optional: &[(&str, GuardrailValue)],
    #[case] rate: usize,
) {
    let sources = StreamFlagSources {
        attributes: flags(attributes),
        guardrail_config: flags(config),
        optional_params: flags(optional),
    };
    let resolved = StreamSettings::resolve(&sources, false, &base(false)).unwrap();
    assert_eq!(resolved.sampling_rate, rate);
}

#[rstest]
#[case::rate_zero("streaming_sampling_rate", GuardrailValue::SignedInteger(0))]
#[case::rate_negative("streaming_sampling_rate", GuardrailValue::SignedInteger(-3))]
#[case::rate_unsigned_zero("streaming_sampling_rate", GuardrailValue::UnsignedInteger(0))]
#[case::rate_text("streaming_sampling_rate", GuardrailValue::Text(String::from("5")))]
#[case::rate_decimal("streaming_sampling_rate", GuardrailValue::Decimal(2.0_f64))]
#[case::end_of_stream_text(
    "streaming_end_of_stream_only",
    GuardrailValue::Text(String::from("true"))
)]
#[case::mode_boolean("streaming_transform_mode", GuardrailValue::Boolean(true))]
#[case::buffer_integer("streaming_buffer_until_moderated", GuardrailValue::SignedInteger(1))]
#[case::release_text(
    "streaming_buffer_release_on_scan",
    GuardrailValue::Text(String::from("yes"))
)]
fn settings_refuse_a_value_of_the_wrong_type(
    #[case] name: &'static str,
    #[case] value: GuardrailValue,
) {
    let sources = StreamFlagSources {
        attributes: BTreeMap::new(),
        guardrail_config: flags(&[(name, value)]),
        optional_params: BTreeMap::new(),
    };
    let resolved = StreamSettings::resolve(&sources, false, &base(false));
    assert_eq!(resolved, Err(InvalidFlag { name }));
}

#[rstest]
#[case::incremental_diff("incremental_diff", TransformMode::IncrementalDiff)]
#[case::block_only("block_only", TransformMode::BlockOnly)]
#[case::unknown_text("Incremental_Diff", TransformMode::BlockOnly)]
fn settings_any_mode_but_incremental_diff_is_block_only(
    #[case] mode: &str,
    #[case] expected: TransformMode,
) {
    let sources = StreamFlagSources {
        attributes: flags(&[(
            "streaming_transform_mode",
            GuardrailValue::Text(mode.to_owned()),
        )]),
        guardrail_config: BTreeMap::new(),
        optional_params: BTreeMap::new(),
    };
    let resolved = StreamSettings::resolve(&sources, false, &base(false)).unwrap();
    assert_eq!(resolved.transform_mode, expected);
}

#[rstest]
#[case::buffering_masked(true, true, false, true)]
#[case::buffering_unmasked(true, false, true, false)]
#[case::masked_without_buffering(false, true, false, false)]
fn settings_mask_response_content_turns_buffering_off(
    #[case] buffer_default: bool,
    #[case] mask: bool,
    #[case] buffering: bool,
    #[case] warned: bool,
) {
    let (logs, _guard) = capture_logs();
    let resolved =
        StreamSettings::resolve(&StreamFlagSources::default(), buffer_default, &base(mask))
            .unwrap();
    assert_eq!(resolved.buffer_until_moderated, buffering);
    assert_eq!(resolved.end_of_stream_only, buffering);
    assert_eq!(logs.contents().contains(MASK_WARNING), warned);
}

#[rstest]
#[case::buffered_without_release(true, false, true)]
#[case::buffered_with_release(true, true, false)]
#[case::live_without_release(false, false, false)]
fn settings_buffering_without_release_forces_end_of_stream_only(
    #[case] buffer: bool,
    #[case] release: bool,
    #[case] end_of_stream_only: bool,
) {
    let sources = StreamFlagSources {
        attributes: BTreeMap::new(),
        guardrail_config: BTreeMap::new(),
        optional_params: flags(&[
            (
                "streaming_buffer_until_moderated",
                GuardrailValue::Boolean(buffer),
            ),
            (
                "streaming_buffer_release_on_scan",
                GuardrailValue::Boolean(release),
            ),
        ]),
    };
    let resolved = StreamSettings::resolve(&sources, false, &base(false)).unwrap();
    assert_eq!(resolved.end_of_stream_only, end_of_stream_only);
}

#[tokio::test]
async fn passthrough_without_guardrail_sends_every_chunk() {
    let mut harness = Harness::new(KeyMode::Texts, Vec::new()).rate(0);
    harness.guarded = false;
    let outputs = harness.run(texts("abc")).await;
    assert_eq!(lines(&outputs), ["a", "b", "c"]);
    assert_eq!(harness.calls(CallType::Completion), scans(&[]));
}

#[tokio::test]
async fn passthrough_closed_gate_sends_every_chunk() {
    let mut harness = Harness::new(KeyMode::Texts, Vec::new()).rate(0);
    harness.base.settings.event_hook = Some(LitellmParamsMode::One(String::from("post_call")));
    let (logs, _guard) = capture_logs();
    let outputs = harness.run(texts("abc")).await;
    assert_eq!(lines(&outputs), ["a", "b", "c"]);
    assert_eq!(harness.calls(CallType::Completion), scans(&[]));
    let disabled =
        format!("UnifiedLLMGuardrails: Post-call streaming scanning disabled for {NAME}");
    assert!(logs.contents().contains(&disabled));
}

#[tokio::test]
async fn passthrough_gate_error_is_the_first_item() {
    let mut harness = Harness::new(KeyMode::Texts, Vec::new());
    harness.base.settings.event_hook = Some(LitellmParamsMode::One(String::from("post_call")));
    harness.request_data.insert(
        String::from("guardrails"),
        GuardrailValue::Text(String::from("x")),
    );
    let outputs = harness.run(texts("ab")).await;
    assert_eq!(lines(&outputs), ["gate error"]);
}

#[tokio::test]
async fn passthrough_invalid_flag_is_the_first_item() {
    let mut harness = Harness::new(KeyMode::Texts, Vec::new()).rate(0);
    let outputs = harness.run(texts("ab")).await;
    assert_eq!(lines(&outputs), ["invalid streaming_sampling_rate"]);
}

#[rstest]
#[case::route_without_translation(Some("/embed"), vec![chunk("a", None), chunk("b", None)], ["a", "b"])]
#[case::chunk_of_unknown_kind(None, vec![GuardrailValue::Text(String::from("x")), chunk("b", None)], ["x", "b"])]
#[tokio::test]
async fn passthrough_unmapped_call_type_sends_every_chunk(
    #[case] route: Option<&'static str>,
    #[case] items: Vec<GuardrailValue>,
    #[case] expected: [&str; 2],
) {
    let mut harness = Harness::new(KeyMode::Texts, Vec::new()).rate(1);
    harness.route = route;
    let outputs = harness.run(items).await;
    assert_eq!(lines(&outputs), expected);
    assert_eq!(harness.calls(CallType::Completion), scans(&[]));
}

// LiteLLM raises IndexError on call_types[0]; an empty list falls back to the chunk inference instead
#[tokio::test]
async fn route_with_empty_call_types_infers_the_translation_from_the_chunk() {
    let mut harness = Harness::new(KeyMode::Texts, Vec::new()).rate(1);
    harness.route = Some("/empty");
    let outputs = harness.run(texts("ab")).await;
    assert_eq!(lines(&outputs), ["a", "b"]);
    assert_eq!(harness.calls(CallType::Completion), scans(&[1, 2]));
}

#[tokio::test]
async fn passthrough_source_error_ends_the_stream() {
    let mut harness = Harness::new(KeyMode::Texts, Vec::new());
    let source = vec![
        Ok(chunk("a", None)),
        Err(SourceFailure),
        Ok(chunk("b", None)),
    ];
    let outputs = harness.run_source(source).await;
    assert_eq!(
        lines(&outputs),
        ["a", "loop error the upstream response stream failed"]
    );
    assert_eq!(harness.calls(CallType::Completion), scans(&[]));
}

#[tokio::test]
async fn sampling_scans_every_nth_chunk() {
    let mut harness = Harness::new(KeyMode::Texts, Vec::new()).rate(2);
    let outputs = harness.run(texts("abcde")).await;
    assert_eq!(lines(&outputs), ["a", "b", "c", "d", "e"]);
    assert_eq!(harness.calls(CallType::Completion), scans(&[2, 4, 5]));
}

#[rstest]
#[case::nothing_new(vec![chunk("a", None), chunk("", None), chunk("b", None)], &[1, 3])]
#[case::nothing_to_scan(vec![chunk("", None), chunk("a", None)], &[2])]
#[tokio::test]
async fn sampling_skips_a_redundant_round(
    #[case] items: Vec<GuardrailValue>,
    #[case] scanned: &[usize],
) {
    let mut harness = Harness::new(KeyMode::Texts, Vec::new()).rate(1);
    let sent: Vec<String> = items.iter().map(content).collect();
    let outputs = harness.run(items).await;
    assert_eq!(lines(&outputs), sent);
    assert_eq!(harness.calls(CallType::Completion), scans(scanned));
}

#[rstest]
#[case::whole_window_scanned(KeyMode::Texts, &["a", "block started=true sent=1:a original=ab"])]
#[case::tool_calls_in_flight(KeyMode::ToolsInFlight, &["block started=false sent=0: original=ab"])]
#[case::missing_key(KeyMode::Missing, &["block started=false sent=0: original=ab"])]
#[tokio::test]
async fn sampling_holds_a_buffered_window_until_a_round_scans_it_all(
    #[case] key: KeyMode,
    #[case] expected: &[&str],
) {
    let mut harness = Harness::new(key, vec![Ok(()), block()])
        .rate(1)
        .flag(
            "streaming_buffer_until_moderated",
            GuardrailValue::Boolean(true),
        )
        .flag(
            "streaming_buffer_release_on_scan",
            GuardrailValue::Boolean(true),
        );
    let outputs = harness.run(texts("ab")).await;
    assert_eq!(lines(&outputs), expected);
}

#[rstest]
#[case::released(KeyMode::Texts, &["a", "", "block started=true sent=2:a original=ab"])]
#[case::held(KeyMode::ToolsInFlight, &["block started=false sent=0: original=ab"])]
#[tokio::test]
async fn sampling_redundant_round_releases_the_window_unless_held(
    #[case] key: KeyMode,
    #[case] expected: &[&str],
) {
    let mut harness = Harness::new(key, vec![Ok(()), block()])
        .rate(1)
        .flag(
            "streaming_buffer_until_moderated",
            GuardrailValue::Boolean(true),
        )
        .flag(
            "streaming_buffer_release_on_scan",
            GuardrailValue::Boolean(true),
        );
    let items = vec![chunk("a", None), chunk("", None), chunk("b", None)];
    let outputs = harness.run(items).await;
    assert_eq!(lines(&outputs), expected);
    assert_eq!(harness.calls(CallType::Completion), scans(&[1, 3]));
}

#[tokio::test]
async fn block_mid_stream_measures_against_the_chunks_sent() {
    let mut harness = Harness::new(KeyMode::Texts, vec![Ok(()), block()]).rate(2);
    let outputs = harness.run(texts("abcde")).await;
    assert_eq!(
        lines(&outputs),
        ["a", "b", "c", "block started=true sent=3:abc original=abcd"]
    );
    assert_eq!(harness.calls(CallType::Completion), scans(&[2, 4]));
}

#[rstest]
#[case::own_original(None, "block started=false sent=0: original=a")]
#[case::given_original(
    Some(chunk("given", None)),
    "block started=false sent=0: original=given"
)]
#[tokio::test]
async fn block_on_the_first_round_has_not_started(
    #[case] original: Option<GuardrailValue>,
    #[case] expected: &str,
) {
    let plans = vec![Err(modify_response("blocked", original))];
    let mut harness = Harness::new(KeyMode::Texts, plans).rate(1);
    let outputs = harness.run(texts("ab")).await;
    assert_eq!(lines(&outputs), [expected]);
    assert_eq!(harness.calls(CallType::Completion), scans(&[1]));
}

#[rstest]
#[case::text_detail(http(400), "error status=400 sent=a")]
#[case::object_detail(Err(GuardrailFailure::HttpObject { status: 403, detail: IndexMap::new() }), "error status=403 sent=a")]
#[tokio::test]
async fn block_http_failure_sends_the_error_items(
    #[case] failure: Result<(), GuardrailFailure>,
    #[case] expected: &str,
) {
    let mut harness = Harness::new(KeyMode::Texts, vec![Ok(()), failure]).rate(1);
    let outputs = harness.run(texts("abc")).await;
    assert_eq!(lines(&outputs), ["a", expected]);
}

#[rstest]
#[case::http_before_any_chunk(
    http(400),
    "loop error the streamed error could not be written into the response stream"
)]
#[case::other_failure(
    Err(GuardrailFailure::Other(String::from("boom"))),
    "loop error the guardrail failed while transforming the streamed response"
)]
#[case::no_block_message(
    Err(modify_response("unwritable", None)),
    "loop error the guardrail blocked the streamed response and no block message could be written"
)]
#[tokio::test]
async fn block_failure_without_stream_output_leaves_as_an_error(
    #[case] failure: Result<(), GuardrailFailure>,
    #[case] expected: &str,
) {
    let mut harness = Harness::new(KeyMode::Texts, vec![failure]).rate(1);
    let outputs = harness.run(texts("ab")).await;
    assert_eq!(lines(&outputs), [expected]);
}

#[rstest]
#[case::until_moderated_pass(false, Ok(()), &["a", "b", "c"])]
#[case::until_moderated_block(false, block(), &["block started=false sent=0: original=abc"])]
#[case::release_on_scan_pass(true, Ok(()), &["a", "b", "c"])]
#[case::release_on_scan_block(true, block(), &["block started=false sent=0: original=abc"])]
#[tokio::test]
async fn end_scan_releases_buffered_chunks_only_after_a_pass(
    #[case] release_on_scan: bool,
    #[case] verdict: Result<(), GuardrailFailure>,
    #[case] expected: &[&str],
) {
    let mut harness = Harness::new(KeyMode::Texts, vec![verdict]).flag(
        "streaming_buffer_release_on_scan",
        GuardrailValue::Boolean(release_on_scan),
    );
    harness.buffer_default = true;
    let outputs = harness.run(texts("abc")).await;
    assert_eq!(lines(&outputs), expected);
    assert_eq!(harness.calls(CallType::Completion), scans(&[3]));
}

#[rstest]
#[case::pass(Ok(()), &["a", "b", "c"])]
#[case::block(block(), &["a", "block started=true sent=1:a original=abc"])]
#[case::http(http(400), &["a", "error status=400 sent=a"])]
#[tokio::test]
async fn end_scan_holds_the_chunks_after_the_stream_ended(
    #[case] verdict: Result<(), GuardrailFailure>,
    #[case] expected: &[&str],
) {
    let mut harness = Harness::new(KeyMode::Texts, vec![verdict]).flag(
        "streaming_end_of_stream_only",
        GuardrailValue::Boolean(true),
    );
    let items = vec![chunk("a", None), chunk("b", Some("stop")), chunk("c", None)];
    let outputs = harness.run(items).await;
    assert_eq!(lines(&outputs), expected);
    assert_eq!(harness.calls(CallType::Completion), scans(&[3]));
}

#[rstest]
#[case::pending(false)]
#[case::buffered(true)]
#[tokio::test]
async fn end_scan_skips_a_redundant_scan(#[case] buffered: bool) {
    let mut harness = Harness::new(KeyMode::Texts, vec![block()]).flag(
        "streaming_end_of_stream_only",
        GuardrailValue::Boolean(true),
    );
    harness.buffer_default = buffered;
    let outputs = harness
        .run(vec![chunk("", None), chunk("", Some("stop"))])
        .await;
    assert_eq!(lines(&outputs), ["", ""]);
    assert_eq!(harness.calls(CallType::Completion), scans(&[]));
}

#[tokio::test]
async fn end_scan_other_failure_leaves_as_an_error() {
    let plans = vec![Err(GuardrailFailure::Other(String::from("boom")))];
    let mut harness = Harness::new(KeyMode::Texts, plans);
    harness.buffer_default = true;
    let outputs = harness.run(texts("ab")).await;
    assert_eq!(
        lines(&outputs),
        ["loop error the guardrail failed while transforming the streamed response"]
    );
}

#[rstest]
#[case::sampled_every_chunk(false, &[1, 2])]
#[case::end_of_stream_only(true, &[2])]
#[tokio::test]
async fn transform_route_incremental_diff_on_a_chat_route_runs_the_transform_loop(
    #[case] end_of_stream_only: bool,
    #[case] scanned: &[usize],
) {
    let mut harness = Harness::new(KeyMode::Texts, Vec::new()).rate(1).flag(
        "streaming_transform_mode",
        GuardrailValue::Text(String::from("incremental_diff")),
    );
    if end_of_stream_only {
        harness = harness.flag(
            "streaming_end_of_stream_only",
            GuardrailValue::Boolean(true),
        );
    }
    harness.route = Some("/chat");
    let (logs, _guard) = capture_logs();
    let items = vec![chunk("a", None), chunk("b", Some("stop"))];
    let outputs = harness.run(items).await;
    let expected: Vec<ScanCall> = scanned
        .iter()
        .map(|count| ScanCall {
            items: *count,
            sink: true,
        })
        .collect();
    assert_eq!(harness.calls(CallType::Completion), expected);
    assert_eq!(lines(&outputs), [""]);
    assert!(!logs.contents().contains(FALLBACK_WARNING));
}

#[tokio::test]
async fn frames_pass_through_in_order_without_a_translation() {
    let mut harness = Harness::new(KeyMode::Texts, Vec::new()).rate(0);
    harness.guarded = false;
    let outputs = harness
        .run_frames(vec![
            StreamItem::Bytes(b"event: one\n\n".to_vec()),
            StreamItem::Json(chunk("a", None)),
            StreamItem::Bytes(b"event: two\n\n".to_vec()),
        ])
        .await;
    assert_eq!(
        lines(&outputs),
        ["bytes event: one\n\n", "a", "bytes event: two\n\n"]
    );
}

#[tokio::test]
async fn frames_bytes_scan_on_the_anthropic_route() {
    let mut harness = Harness::new(KeyMode::Texts, Vec::new()).rate(1);
    harness.registry.entries.push(TranslationEntry {
        call_type: CallType::AnthropicMessages,
        translation: fake(KeyMode::Texts, Vec::new()),
        chat_completions: false,
    });
    harness.route = Some("/v1/messages");
    let frame = b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n".to_vec();
    let outputs = harness.run_frames(vec![StreamItem::Bytes(frame)]).await;
    assert_eq!(harness.calls(CallType::AnthropicMessages), scans(&[1]));
    assert_eq!(lines(&outputs).len(), 1);
}

#[tokio::test]
async fn frames_block_after_a_bytes_prefix_keeps_the_frames_in_the_original() {
    let plans = vec![Err(modify_response("blocked", None))];
    let mut harness = Harness::new(KeyMode::Texts, plans).rate(2);
    harness.route = Some("/chat");
    let outputs = harness
        .run_frames(vec![
            StreamItem::Bytes(b"event: first\n\n".to_vec()),
            StreamItem::Bytes(b"event: second\n\n".to_vec()),
        ])
        .await;
    let expected =
        "block started=true sent=1:event: first\n\n original=event: first\n\nevent: second\n\n";
    assert_eq!(lines(&outputs), ["bytes event: first\n\n", expected]);
}

#[tokio::test]
async fn frames_http_error_keeps_the_anthropic_bytes_frame() {
    let built = block_continuation::build_stream_error_items("blocked by policy").unwrap();
    let frames: Vec<StreamItem> = built.into_iter().map(StreamItem::Bytes).collect();
    let mut harness = Harness::new(KeyMode::Texts, Vec::new()).rate(1);
    harness.registry.entries.push(TranslationEntry {
        call_type: CallType::AnthropicMessages,
        translation: fake(KeyMode::Texts, vec![Ok(()), http(405)]),
        chat_completions: false,
    });
    harness.error_items(&frames);
    harness.route = Some("/v1/messages");
    let outputs = harness
        .run_frames(vec![
            StreamItem::Bytes(b"event: first\n\n".to_vec()),
            StreamItem::Bytes(b"event: second\n\n".to_vec()),
        ])
        .await;
    assert_eq!(lines(&outputs).len(), 2);
    assert_eq!(lines(&outputs)[0], "bytes event: first\n\n");
    assert_eq!(outputs[1], Ok(StreamOutput::Item(frames[0].clone())));
    let Ok(StreamOutput::Item(StreamItem::Bytes(frame))) = &outputs[1] else {
        panic!("expected the raw error frame, got {:?}", outputs[1]);
    };
    assert!(
        String::from_utf8(frame.clone())
            .unwrap()
            .contains("blocked by policy")
    );
}

#[rstest]
#[case::route_without_chat_translation(Some("/responses"), CallType::Responses)]
#[case::no_route(None, CallType::Completion)]
#[tokio::test]
async fn transform_route_falls_back_to_block_only_with_a_warning(
    #[case] route: Option<&'static str>,
    #[case] scanned: CallType,
) {
    let mut harness = Harness::new(KeyMode::Texts, Vec::new()).flag(
        "streaming_transform_mode",
        GuardrailValue::Text(String::from("incremental_diff")),
    );
    harness.route = route;
    let (logs, _guard) = capture_logs();
    let outputs = harness.run(texts("ab")).await;
    assert_eq!(lines(&outputs), ["a", "b"]);
    assert_eq!(harness.calls(scanned), scans(&[2]));
    assert!(logs.contents().contains(FALLBACK_WARNING));
}
