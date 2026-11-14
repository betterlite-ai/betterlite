#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_openai::chat::{FunctionCall, ToolCall};
use rstest::rstest;

use crate::adaptive_classification::constants::{
    MIN_TURNS_FOR_CLEAN_CREDIT, TOOL_CALL_HISTORY_MAX,
};
use crate::adaptive_classification::request_type::RequestType;
use crate::feedback_signals::apply_turn;
use crate::feedback_signals::detector::{
    Detector, DetectorThresholds, detect_response_signals, detect_user_feedback,
};
use crate::feedback_signals::error::SignalError;
use crate::feedback_signals::signal::{SignalDelta, apply_signal_delta, merge_signal_deltas};
use crate::feedback_signals::state::SessionState;
use crate::feedback_signals::turn::{ToolResult, Turn};
use crate::signal_math::{jaccard, tokens};

const NONE: [i64; 7] = [0; 7];
const MISALIGNED: [i64; 7] = [1, 0, 0, 0, 0, 0, 0];
const STAGNANT: [i64; 7] = [0, 1, 0, 0, 0, 0, 0];
const DISENGAGED: [i64; 7] = [0, 0, 1, 0, 0, 0, 0];
const SATISFIED: [i64; 7] = [0, 0, 0, 1, 0, 0, 0];
const FAILED: [i64; 7] = [0, 0, 0, 0, 1, 0, 0];
const LOOPED: [i64; 7] = [0, 0, 0, 0, 0, 1, 0];
const EXHAUSTED: [i64; 7] = [0, 0, 0, 0, 0, 0, 1];

fn detector() -> Detector {
    Detector::new(DetectorThresholds::PINNED).unwrap()
}

fn fields(delta: SignalDelta) -> [i64; 7] {
    [
        delta.misalignment,
        delta.stagnation,
        delta.disengagement,
        delta.satisfaction,
        delta.failure,
        delta.r#loop,
        delta.exhaustion,
    ]
}

fn counters(state: &SessionState) -> [i64; 7] {
    [
        state.misalignment_count,
        state.stagnation_count,
        state.disengagement_count,
        state.satisfaction_count,
        state.failure_count,
        state.loop_count,
        state.exhaustion_count,
    ]
}

fn call(name: &str, arguments: &str) -> ToolCall {
    ToolCall {
        id: String::new(),
        kind: "function".to_owned(),
        function: FunctionCall {
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        },
    }
}

fn tool_result_of(content: &str, is_error: bool) -> ToolResult {
    ToolResult {
        content: Some(content.to_owned()),
        is_error,
    }
}

fn state() -> SessionState {
    SessionState::new(
        "session".to_owned(),
        "router".to_owned(),
        "model".to_owned(),
        RequestType::General,
    )
}

fn make_turn(
    user: Option<&str>,
    assistant: Option<&str>,
    tool_results: Vec<ToolResult>,
    response_status: Option<i64>,
) -> Turn {
    Turn {
        user_content: user.map(str::to_owned),
        assistant_content: assistant.map(str::to_owned),
        tool_calls: Vec::new(),
        tool_results,
        response_status,
    }
}

fn assert_feedback(
    previous: Option<&str>,
    current: Option<&str>,
    tool: Option<(&str, bool)>,
    allow_satisfaction: bool,
    expected: [i64; 7],
) {
    let results = tool.map_or_else(Vec::new, |(content, is_error)| {
        vec![tool_result_of(content, is_error)]
    });
    let turn = make_turn(current, None, results, None);
    let delta = detect_user_feedback(&detector(), previous, &turn, allow_satisfaction);
    assert_eq!(fields(delta), expected);
}

fn assert_response(
    previous: Option<&str>,
    assistant: &str,
    status: Option<i64>,
    expected: [i64; 7],
) {
    let turn = make_turn(None, Some(assistant), Vec::new(), status);
    let delta = detect_response_signals(&detector(), previous, &[], &turn);
    assert_eq!(fields(delta), expected);
}

fn assert_loop(history: &[&str], calls: &[(&str, &str)], expected: [i64; 7]) {
    let known: Vec<String> = history.iter().map(|entry| (*entry).to_owned()).collect();
    let new_calls: Vec<ToolCall> = calls
        .iter()
        .map(|(name, arguments)| call(name, arguments))
        .collect();
    let turn = Turn {
        user_content: None,
        assistant_content: None,
        tool_calls: new_calls,
        tool_results: Vec::new(),
        response_status: None,
    };
    let delta = detect_response_signals(&detector(), None, &known, &turn);
    assert_eq!(fields(delta), expected);
}

fn assert_exhaustion(
    content: Option<&str>,
    is_error: bool,
    status: Option<i64>,
    expected: [i64; 7],
) {
    let result = ToolResult {
        content: content.map(str::to_owned),
        is_error,
    };
    let turn = make_turn(None, None, vec![result], status);
    let delta = detect_response_signals(&detector(), None, &[], &turn);
    assert_eq!(fields(delta), expected);
}

fn assert_tokens(text: Option<&str>, expected: &[&str]) {
    let found = tokens(text);
    let words: Vec<&str> = found.iter().map(String::as_str).collect();
    assert_eq!(words, expected);
}

fn assert_jaccard(left: &[&str], right: &[&str], expected: f64) {
    let left_tokens: BTreeSet<String> = left.iter().map(|word| (*word).to_owned()).collect();
    let right_tokens: BTreeSet<String> = right.iter().map(|word| (*word).to_owned()).collect();
    assert_eq!(
        jaccard(&left_tokens, &right_tokens).to_bits(),
        expected.to_bits()
    );
}

fn assert_counted() {
    let one = SignalDelta {
        misalignment: 1,
        stagnation: 0,
        disengagement: 0,
        satisfaction: 0,
        failure: 0,
        r#loop: 0,
        exhaustion: 0,
    };
    let all = SignalDelta {
        misalignment: 1,
        stagnation: 1,
        disengagement: 1,
        satisfaction: 1,
        failure: 1,
        r#loop: 1,
        exhaustion: 1,
    };
    assert!(!SignalDelta::default().any_fired());
    assert!(one.any_fired());
    assert_eq!(fields(merge_signal_deltas(&[])), NONE);
    assert_eq!(fields(merge_signal_deltas(&[all, all])), [2; 7]);
    let mut state = state();
    assert_eq!(state.last_processed_turn, -1);
    assert!(!state.clean_credit_awarded);
    assert_eq!(state.terminal_status, None);
    apply_signal_delta(&mut state, all);
    apply_signal_delta(&mut state, all);
    assert_eq!(counters(&state), [2; 7]);
}

fn assert_credit_once() {
    let detector = detector();
    let mut state = state();
    let first = apply_turn(
        &detector,
        &mut state,
        &make_turn(Some("hello there"), Some("hi"), Vec::new(), Some(200)),
    );
    assert_eq!(fields(first), NONE);
    let second = apply_turn(
        &detector,
        &mut state,
        &make_turn(Some("thx"), Some("sure"), Vec::new(), Some(200)),
    );
    assert_eq!(fields(second), NONE);
    let third = apply_turn(
        &detector,
        &mut state,
        &make_turn(Some("nice"), Some("ok"), Vec::new(), Some(200)),
    );
    assert_eq!(fields(third), SATISFIED);
    assert!(state.clean_credit_awarded);
    let fourth = apply_turn(
        &detector,
        &mut state,
        &make_turn(Some("perfect"), Some("good"), Vec::new(), Some(200)),
    );
    assert_eq!(fields(fourth), NONE);
    let fifth = apply_turn(
        &detector,
        &mut state,
        &make_turn(Some("thanks"), Some("welcome"), Vec::new(), Some(200)),
    );
    assert_eq!(fields(fifth), NONE);
    assert_eq!(counters(&state), [0, 0, 0, 1, 0, 0, 0]);
    assert_eq!(state.last_user_content.as_deref(), Some("thanks"));
    assert_eq!(state.last_assistant_content.as_deref(), Some("welcome"));
    assert_eq!(state.turn_count, 5);
    assert_eq!(state.last_processed_turn, 5);
    assert_eq!(state.terminal_status, Some(200));
}

fn assert_advance_bounds() {
    let calls: Vec<ToolCall> = (0..TOOL_CALL_HISTORY_MAX)
        .map(|index| call(&format!("tool{index}"), "{}"))
        .collect();
    let turn = Turn {
        user_content: Some(String::new()),
        assistant_content: Some(String::new()),
        tool_calls: calls,
        tool_results: Vec::new(),
        response_status: Some(200),
    };
    let mut state = state();
    let delta = apply_turn(&detector(), &mut state, &turn);
    assert_eq!(fields(delta), NONE);
    assert_eq!(state.tool_call_history.len(), TOOL_CALL_HISTORY_MAX);
    assert_eq!(
        state.tool_call_history.first().map(String::as_str),
        Some("tool0({})")
    );
    assert_eq!(state.last_user_content, None);
    assert_eq!(state.last_assistant_content, None);
    assert_eq!(state.terminal_status, Some(200));
    assert_eq!(state.turn_count, 1);
    assert_eq!(state.last_processed_turn, 1);
}

#[derive(Clone, Copy)]
enum Behavior {
    Feedback(
        Option<&'static str>,
        Option<&'static str>,
        Option<(&'static str, bool)>,
        bool,
        [i64; 7],
    ),
    Response(Option<&'static str>, &'static str, Option<i64>, [i64; 7]),
    Loop(
        &'static [&'static str],
        &'static [(&'static str, &'static str)],
        [i64; 7],
    ),
    Exhaustion(Option<&'static str>, bool, Option<i64>, [i64; 7]),
    Tokens(Option<&'static str>, &'static [&'static str]),
    Jaccard(&'static [&'static str], &'static [&'static str], f64),
    Counted,
    CreditOnce,
    AdvanceBounds,
}

#[rstest]
#[case::misalignment_rephrased(Behavior::Feedback(
    Some("Hello there"),
    Some("hello world"),
    None,
    false,
    MISALIGNED
))]
#[case::misalignment_topic_changed(Behavior::Feedback(
    Some("hello there"),
    Some("goodbye moon"),
    None,
    false,
    NONE
))]
#[case::misalignment_repeated(Behavior::Feedback(
    Some("hello there"),
    Some("hello there"),
    None,
    false,
    NONE
))]
#[case::disengagement_phrase(Behavior::Feedback(
    None,
    Some("this isn't working, abort"),
    None,
    false,
    DISENGAGED
))]
#[case::satisfaction_without_allow(Behavior::Feedback(
    None,
    Some("thanks, that worked"),
    None,
    false,
    NONE
))]
#[case::satisfaction_allowed(Behavior::Feedback(None, Some("nice"), None, true, SATISFIED))]
#[case::tool_failure(Behavior::Feedback(
    None,
    Some("run it"),
    Some(("boom", true)),
    false,
    FAILED
))]
#[case::empty_tool_result_is_not_failure(Behavior::Feedback(
    None,
    Some("run it"),
    Some(("", false)),
    false,
    NONE
))]
#[case::stagnation_near_duplicate(Behavior::Response(
    Some("red green blue"),
    "red green yellow",
    None,
    STAGNANT
))]
#[case::stagnation_rewritten(Behavior::Response(
    Some("red green blue"),
    "purple orange",
    None,
    NONE
))]
#[case::exhaustion_status(Behavior::Response(None, "busy", Some(429), EXHAUSTED))]
#[case::loop_after_repeats(Behavior::Loop(
    &["shell({})", "shell({})"], &[("shell", "{}")], LOOPED
))]
#[case::loop_different_signature(Behavior::Loop(
    &["shell({})", "shell({})"], &[("shell", "{\"a\":1}")], NONE
))]
#[case::exhaustion_keyword(Behavior::Exhaustion(
    Some("CONTEXT WINDOW is full"),
    false,
    Some(200),
    EXHAUSTED
))]
#[case::tool_error_is_not_exhaustion(Behavior::Exhaustion(Some("boom"), true, Some(200), NONE))]
#[case::tokens_mixed_text(Behavior::Tokens(
    Some("Hello, world! 123"), &["123", "hello", "world"]
))]
#[case::tokens_absent(Behavior::Tokens(None, &[]))]
#[case::tokens_non_ascii_split(Behavior::Tokens(Some("Café"), &["caf"]))]
#[case::jaccard_identical(Behavior::Jaccard(&["a", "b"], &["a", "b"], 1.0))]
#[case::jaccard_disjoint(Behavior::Jaccard(&["a"], &["b"], 0.0))]
#[case::jaccard_empty(Behavior::Jaccard(&[], &[], 0.0))]
#[case::jaccard_half(Behavior::Jaccard(&["a", "b"], &["a", "c"], 0.5))]
#[case::counted(Behavior::Counted)]
#[case::credit_once(Behavior::CreditOnce)]
#[case::advance_bounds(Behavior::AdvanceBounds)]
#[case::satisfaction_not_matched(Behavior::Feedback(None, Some("run it"), None, true, NONE))]
#[case::absent_user_content(Behavior::Feedback(Some("hello there"), None, None, true, NONE))]
#[case::exhaustion_absent_content(Behavior::Exhaustion(None, false, None, NONE))]
fn feedback_signals_behaviors(#[case] behavior: Behavior) {
    match behavior {
        Behavior::Feedback(previous, current, tool, allow, expected) => {
            assert_feedback(previous, current, tool, allow, expected);
        }
        Behavior::Response(previous, assistant, status, expected) => {
            assert_response(previous, assistant, status, expected);
        }
        Behavior::Loop(history, calls, expected) => assert_loop(history, calls, expected),
        Behavior::Exhaustion(content, is_error, status, expected) => {
            assert_exhaustion(content, is_error, status, expected);
        }
        Behavior::Tokens(text, expected) => assert_tokens(text, expected),
        Behavior::Jaccard(left, right, expected) => assert_jaccard(left, right, expected),
        Behavior::Counted => assert_counted(),
        Behavior::CreditOnce => assert_credit_once(),
        Behavior::AdvanceBounds => assert_advance_bounds(),
    }
}

#[derive(Clone, Copy)]
enum Boundary {
    Threshold(f64, f64, Option<(&'static str, f64)>),
    Misalignment(&'static str, &'static str, bool),
    Stagnation(&'static str, &'static str, bool),
    Loop(usize, bool),
    History(usize, usize, &'static str),
    Gate(&'static str, GatePhase),
    Status(Option<i64>, bool),
}

#[derive(Clone, Copy)]
enum GatePhase {
    Before,
    At,
    WithCredit,
}

fn assert_threshold(misalignment: f64, stagnation: f64, rejected: Option<(&str, f64)>) {
    let outcome = DetectorThresholds::checked(misalignment, stagnation);
    let Some((field, value)) = rejected else {
        assert!(outcome.is_ok(), "finite thresholds must be accepted");
        return;
    };
    let Err(SignalError::NonFinite {
        field: reported,
        value: reported_value,
    }) = outcome
    else {
        panic!("a non-finite {field} threshold was accepted");
    };
    assert_eq!(reported, field);
    assert_eq!(reported_value.to_bits(), value.to_bits());
}

fn assert_misalignment(previous: &str, current: &str, fires: bool) {
    let turn = make_turn(Some(current), None, Vec::new(), None);
    let delta = detect_user_feedback(&detector(), Some(previous), &turn, false);
    assert_eq!(delta.misalignment != 0, fires);
}

fn assert_stagnation(previous: &str, current: &str, fires: bool) {
    let turn = make_turn(None, Some(current), Vec::new(), None);
    let delta = detect_response_signals(&detector(), Some(previous), &[], &turn);
    assert_eq!(delta.stagnation != 0, fires);
}

fn assert_loop_boundary(repeats: usize, fires: bool) {
    let history: Vec<String> = (0..repeats).map(|_| "shell({})".to_owned()).collect();
    let turn = Turn {
        user_content: None,
        assistant_content: None,
        tool_calls: vec![call("shell", "{}")],
        tool_results: Vec::new(),
        response_status: None,
    };
    let delta = detect_response_signals(&detector(), None, &history, &turn);
    assert_eq!(delta.r#loop != 0, fires);
}

fn assert_history(calls: usize, kept: usize, first: &str) {
    let tool_calls: Vec<ToolCall> = (0..calls)
        .map(|index| call(&format!("tool{index}"), "{}"))
        .collect();
    let turn = Turn {
        user_content: Some(String::new()),
        assistant_content: Some(String::new()),
        tool_calls,
        tool_results: Vec::new(),
        response_status: None,
    };
    let mut state = state();
    let delta = apply_turn(&detector(), &mut state, &turn);
    assert_eq!(fields(delta), NONE);
    assert_eq!(state.tool_call_history.len(), kept);
    let expected = (calls != 0).then_some(first);
    assert_eq!(
        state.tool_call_history.first().map(String::as_str),
        expected
    );
}

fn assert_gate(phrase: &str, phase: GatePhase) {
    let (offset, clean_credit, fires) = match phase {
        GatePhase::Before => (2, false, false),
        GatePhase::At => (1, false, true),
        GatePhase::WithCredit => (1, true, false),
    };
    let mut state = state();
    state.turn_count = i64::from(MIN_TURNS_FOR_CLEAN_CREDIT).saturating_sub(offset);
    state.clean_credit_awarded = clean_credit;
    let turn = make_turn(Some(phrase), None, Vec::new(), None);
    let delta = apply_turn(&detector(), &mut state, &turn);
    assert_eq!(delta.satisfaction != 0, fires);
}

#[rstest]
#[case::threshold_nan(Boundary::Threshold(
    f64::NAN, 0.5, Some(("misalignment", f64::NAN))
))]
#[case::threshold_infinite(Boundary::Threshold(
    0.45, f64::INFINITY, Some(("stagnation", f64::INFINITY))
))]
#[case::threshold_finite(Boundary::Threshold(0.45, 0.5, None))]
#[case::misalignment_at_threshold(Boundary::Misalignment(
    "a1 a2 a3 a4 a5 a6 a7 a8 a9 a10 a11 a12 a13 a14 a15 a16 a17 a18 a19 a20",
    "a1 a2 a3 a4 a5 a6 a7 a8 a9 b1 b2 b3 b4 b5 b6 b7 b8 b9 b10 b11",
    false,
))]
#[case::misalignment_just_below(Boundary::Misalignment(
    "alpha beta gamma delta epsilon zeta",
    "alpha beta gamma delta eta theta iota",
    true,
))]
#[case::misalignment_just_above(Boundary::Misalignment(
    "p1 p2 p3 p4 p5 q1 q2 q3",
    "p1 p2 p3 p4 p5 r1 r2 r3",
    false,
))]
#[case::misalignment_disjoint(Boundary::Misalignment("hello", "world", false))]
#[case::misalignment_identical(Boundary::Misalignment("hello there", "hello there", false))]
#[case::misalignment_empty_current(Boundary::Misalignment("hello", "", false))]
#[case::stagnation_at_threshold(Boundary::Stagnation("red green blue", "red green yellow", true,))]
#[case::stagnation_just_below(Boundary::Stagnation(
    "alpha beta gamma delta epsilon zeta",
    "alpha beta gamma delta eta theta iota",
    false,
))]
#[case::stagnation_empty(Boundary::Stagnation("", "", false))]
#[case::stagnation_identical(Boundary::Stagnation("red green blue", "red green blue", true))]
#[case::loop_one_repeat(Boundary::Loop(1, false))]
#[case::loop_two_repeats(Boundary::Loop(2, true))]
#[case::history_at_cap(Boundary::History(
    TOOL_CALL_HISTORY_MAX,
    TOOL_CALL_HISTORY_MAX,
    "tool0({})",
))]
#[case::history_over_cap(Boundary::History(
    TOOL_CALL_HISTORY_MAX.saturating_add(1),
    TOOL_CALL_HISTORY_MAX,
    "tool1({})",
))]
#[case::history_empty(Boundary::History(0, 0, ""))]
#[case::gate_before_threshold(Boundary::Gate("thanks", GatePhase::Before))]
#[case::gate_at_threshold(Boundary::Gate("thanks", GatePhase::At))]
#[case::gate_clean_credit(Boundary::Gate("thanks", GatePhase::WithCredit))]
#[case::status_exhaustion(Boundary::Status(Some(413), true))]
#[case::status_other(Boundary::Status(Some(500), false))]
#[case::status_absent(Boundary::Status(None, false))]
fn feedback_signals_boundaries(#[case] boundary: Boundary) {
    match boundary {
        Boundary::Threshold(misalignment, stagnation, rejected) => {
            assert_threshold(misalignment, stagnation, rejected);
        }
        Boundary::Misalignment(previous, current, fires) => {
            assert_misalignment(previous, current, fires);
        }
        Boundary::Stagnation(previous, current, fires) => {
            assert_stagnation(previous, current, fires);
        }
        Boundary::Loop(repeats, fires) => assert_loop_boundary(repeats, fires),
        Boundary::History(calls, kept, first) => assert_history(calls, kept, first),
        Boundary::Gate(phrase, phase) => assert_gate(phrase, phase),
        Boundary::Status(status, fires) => {
            let turn = make_turn(None, None, Vec::new(), status);
            let delta = detect_response_signals(&detector(), None, &[], &turn);
            assert_eq!(delta.exhaustion != 0, fires);
        }
    }
}
