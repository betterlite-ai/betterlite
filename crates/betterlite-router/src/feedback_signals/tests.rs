#![cfg(test)]

use std::collections::BTreeSet;
use std::iter::repeat_with;

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
    assistant: Option<&str>,
    results: Vec<ToolResult>,
    status: Option<i64>,
    expected: [i64; 7],
) {
    let turn = make_turn(None, assistant, results, status);
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
    assert_response(None, None, vec![result], status, expected);
}

fn assert_results(results: &[(&str, bool)], expected: [i64; 7]) {
    let tool_results = results
        .iter()
        .map(|(content, is_error)| tool_result_of(content, *is_error))
        .collect();
    let turn = make_turn(None, None, tool_results, None);
    let mut state = state();
    assert_eq!(fields(apply_turn(&detector(), &mut state, &turn)), expected);
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

fn delta_with_one(index: usize) -> SignalDelta {
    let mut counts = [0; 7];
    counts[index] = 1;
    SignalDelta {
        misalignment: counts[0],
        stagnation: counts[1],
        disengagement: counts[2],
        satisfaction: counts[3],
        failure: counts[4],
        r#loop: counts[5],
        exhaustion: counts[6],
    }
}

fn assert_each_field_fires() {
    let singles: Vec<SignalDelta> = (0..7).map(delta_with_one).collect();
    assert!(
        singles.iter().all(SignalDelta::any_fired),
        "any_fired must report every counter"
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
    assert_each_field_fires();
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

fn assert_credit_turn(state: &mut SessionState, user: &str, assistant: &str, expected: [i64; 7]) {
    assert_eq!(
        fields(apply_turn(
            &detector(),
            state,
            &make_turn(Some(user), Some(assistant), Vec::new(), Some(200)),
        )),
        expected
    );
}

fn assert_credit_once() {
    let mut state = state();
    assert_credit_turn(&mut state, "hello there", "hi", NONE);
    assert_credit_turn(&mut state, "thx", "sure", NONE);
    assert_credit_turn(&mut state, "nice", "ok", SATISFIED);
    assert!(state.clean_credit_awarded);
    assert_credit_turn(&mut state, "perfect", "good", NONE);
    assert_credit_turn(&mut state, "thanks", "welcome", NONE);
    assert_eq!(counters(&state), [0, 0, 0, 1, 0, 0, 0]);
    assert_eq!(state.last_user_content.as_deref(), Some("thanks"));
    assert_eq!(state.last_assistant_content.as_deref(), Some("welcome"));
    assert_eq!(state.turn_count, 5);
    assert_eq!(state.last_processed_turn, 5);
    assert_eq!(state.terminal_status, Some(200));
}

fn assert_threaded() {
    let mut state = state();
    let first = make_turn(
        Some("hello there"),
        Some("red green blue"),
        Vec::new(),
        Some(200),
    );
    assert_eq!(fields(apply_turn(&detector(), &mut state, &first)), NONE);
    let second = make_turn(
        Some("hello world"),
        Some("red green yellow"),
        Vec::new(),
        None,
    );
    assert_eq!(
        fields(apply_turn(&detector(), &mut state, &second)),
        [1, 1, 0, 0, 0, 0, 0]
    );
    assert_eq!(state.terminal_status, Some(200));
    let third = make_turn(
        Some("unrelated text"),
        Some("purple orange"),
        Vec::new(),
        None,
    );
    assert_eq!(fields(apply_turn(&detector(), &mut state, &third)), NONE);
    assert_eq!(state.last_user_content.as_deref(), Some("unrelated text"));
    assert_eq!(
        state.last_assistant_content.as_deref(),
        Some("purple orange")
    );
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
enum Case {
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
    Results(&'static [(&'static str, bool)], [i64; 7]),
    Tokens(Option<&'static str>, &'static [&'static str]),
    Jaccard(&'static [&'static str], &'static [&'static str], f64),
    Counted,
    CreditOnce,
    Threaded,
    AdvanceBounds,
    BoundaryThreshold(f64, f64, Option<(&'static str, f64)>),
    BoundaryMisalignment(&'static str, &'static str, bool),
    BoundaryStagnation(&'static str, &'static str, bool),
    BoundaryLoop(usize, bool),
    BoundaryHistory(usize, usize, &'static str),
    BoundaryGate(&'static str, GatePhase),
    BoundaryStatus(Option<i64>, bool),
}

impl Case {
    fn run(self) {
        match self {
            Self::Feedback(previous, current, tool, allow, expected) => {
                assert_feedback(previous, current, tool, allow, expected);
            }
            Self::Response(previous, assistant, status, expected) => {
                assert_response(previous, Some(assistant), Vec::new(), status, expected);
            }
            Self::Loop(history, calls, expected) => assert_loop(history, calls, expected),
            Self::Exhaustion(content, is_error, status, expected) => {
                assert_exhaustion(content, is_error, status, expected);
            }
            Self::Results(results, expected) => assert_results(results, expected),
            Self::Tokens(text, expected) => assert_tokens(text, expected),
            Self::Jaccard(left, right, expected) => assert_jaccard(left, right, expected),
            Self::Counted => assert_counted(),
            Self::CreditOnce => assert_credit_once(),
            Self::Threaded => assert_threaded(),
            Self::AdvanceBounds => assert_advance_bounds(),
            Self::BoundaryThreshold(misalignment, stagnation, rejected) => {
                assert_threshold(misalignment, stagnation, rejected);
            }
            Self::BoundaryMisalignment(previous, current, fires) => {
                assert_misalignment(previous, current, fires);
            }
            Self::BoundaryStagnation(previous, current, fires) => {
                assert_stagnation(previous, current, fires);
            }
            Self::BoundaryLoop(repeats, fires) => assert_loop_boundary(repeats, fires),
            Self::BoundaryHistory(calls, kept, first) => assert_history(calls, kept, first),
            Self::BoundaryGate(text, phase) => assert_gate(text, phase),
            Self::BoundaryStatus(status, fires) => {
                let turn = make_turn(None, None, Vec::new(), status);
                let delta = detect_response_signals(&detector(), None, &[], &turn);
                assert_eq!(delta.exhaustion != 0, fires);
            }
        }
    }
}

#[rstest]
#[case::misalignment_rephrased(Case::Feedback(
    Some("Hello there"),
    Some("hello world"),
    None,
    false,
    MISALIGNED
))]
#[case::misalignment_topic_changed(Case::Feedback(
    Some("hello there"),
    Some("goodbye moon"),
    None,
    false,
    NONE
))]
#[case::misalignment_repeated(Case::Feedback(
    Some("hello there"),
    Some("hello there"),
    None,
    false,
    NONE
))]
#[case::disengagement_phrase(Case::Feedback(
    None,
    Some("this isn't working, abort"),
    None,
    false,
    DISENGAGED
))]
#[case::satisfaction_without_allow(Case::Feedback(
    None,
    Some("thanks, that worked"),
    None,
    false,
    NONE
))]
#[case::satisfaction_allowed(Case::Feedback(None, Some("nice"), None, true, SATISFIED))]
#[case::tool_failure(Case::Feedback(
    None,
    Some("run it"),
    Some(("boom", true)),
    false,
    FAILED
))]
#[case::empty_tool_result_is_not_failure(Case::Feedback(
    None,
    Some("run it"),
    Some(("", false)),
    false,
    NONE
))]
#[case::stagnation_near_duplicate(Case::Response(
    Some("red green blue"),
    "red green yellow",
    None,
    STAGNANT
))]
#[case::stagnation_rewritten(Case::Response(Some("red green blue"), "purple orange", None, NONE))]
#[case::exhaustion_status(Case::Response(None, "busy", Some(429), EXHAUSTED))]
#[case::loop_after_repeats(Case::Loop(
    &["shell({})", "shell({})"], &[("shell", "{}")], LOOPED
))]
#[case::loop_different_signature(Case::Loop(
    &["shell({})", "shell({})"], &[("shell", "{\"a\":1}")], NONE
))]
#[case::exhaustion_keyword(Case::Exhaustion(
    Some("CONTEXT WINDOW is full"),
    false,
    Some(200),
    EXHAUSTED
))]
#[case::tool_error_is_not_exhaustion(Case::Exhaustion(Some("boom"), true, Some(200), NONE))]
#[case::tokens_mixed_text(Case::Tokens(
    Some("Hello, world! 123"), &["123", "hello", "world"]
))]
#[case::tokens_absent(Case::Tokens(None, &[]))]
#[case::tokens_non_ascii_split(Case::Tokens(Some("R\u{e9}sum\u{e9}"), &["r", "sum"]))]
#[case::jaccard_identical(Case::Jaccard(&["a", "b"], &["a", "b"], 1.0))]
#[case::jaccard_disjoint(Case::Jaccard(&["a"], &["b"], 0.0))]
#[case::jaccard_empty(Case::Jaccard(&[], &[], 0.0))]
#[case::jaccard_half(Case::Jaccard(&["a", "b", "c"], &["a", "b", "d"], 0.5))]
#[case::counted(Case::Counted)]
#[case::credit_once(Case::CreditOnce)]
#[case::advance_bounds(Case::AdvanceBounds)]
#[case::satisfaction_not_matched(Case::Feedback(None, Some("run it"), None, true, NONE))]
#[case::absent_user_content(Case::Feedback(Some("hello there"), None, None, true, NONE))]
#[case::exhaustion_absent_content(Case::Exhaustion(None, false, None, NONE))]
#[case::disengagement_cancel(Case::Feedback(
    None,
    Some("cancel this run"),
    None,
    false,
    DISENGAGED
))]
#[case::disengagement_manual(Case::Feedback(
    None,
    Some("I'll do it manually"),
    None,
    false,
    DISENGAGED
))]
#[case::exhaustion_status_504(Case::Response(None, "busy", Some(504), EXHAUSTED))]
#[case::exhaustion_keyword_token_limit(Case::Exhaustion(
    Some("token limit reached"),
    false,
    Some(200),
    EXHAUSTED
))]
#[case::exhaustion_keyword_rate_limit(Case::Exhaustion(
    Some("rate limit exceeded"),
    false,
    Some(200),
    EXHAUSTED
))]
#[case::exhaustion_keyword_too_many_requests(Case::Exhaustion(
    Some("Too Many Requests"),
    false,
    Some(200),
    EXHAUSTED
))]
#[case::exhaustion_keyword_timeout(Case::Exhaustion(
    Some("timeout calling the provider"),
    false,
    Some(200),
    EXHAUSTED
))]
#[case::satisfaction_that_worked(Case::Feedback(None, Some("that worked"), None, true, SATISFIED))]
#[case::satisfaction_that_did_it(Case::Feedback(None, Some("that did it"), None, true, SATISFIED))]
#[case::satisfaction_works_now(Case::Feedback(None, Some("works now"), None, true, SATISFIED))]
#[case::satisfaction_fixed_it(Case::Feedback(None, Some("fixed it"), None, true, SATISFIED))]
#[case::satisfaction_solved_it(Case::Feedback(None, Some("solved it"), None, true, SATISFIED))]
#[case::satisfaction_thank_you(Case::Feedback(None, Some("thank you"), None, true, SATISFIED))]
#[case::satisfaction_thx(Case::Feedback(None, Some("thx"), None, true, SATISFIED))]
#[case::satisfaction_appreciated(Case::Feedback(None, Some("appreciated"), None, true, SATISFIED))]
#[case::satisfaction_appreciate_it(Case::Feedback(
    None,
    Some("appreciate it"),
    None,
    true,
    SATISFIED
))]
#[case::satisfaction_perfect(Case::Feedback(None, Some("perfect"), None, true, SATISFIED))]
#[case::satisfaction_great(Case::Feedback(None, Some("great"), None, true, SATISFIED))]
#[case::satisfaction_excellent(Case::Feedback(None, Some("excellent"), None, true, SATISFIED))]
#[case::satisfaction_exactly(Case::Feedback(None, Some("exactly"), None, true, SATISFIED))]
#[case::disengagement_forget_it(Case::Feedback(None, Some("forget it"), None, false, DISENGAGED))]
#[case::disengagement_never_mind(Case::Feedback(
    None,
    Some("never mind"),
    None,
    false,
    DISENGAGED
))]
#[case::disengagement_give_up(Case::Feedback(None, Some("give up"), None, false, DISENGAGED))]
#[case::disengagement_talk_to_a_human(Case::Feedback(
    None,
    Some("talk to a human"),
    None,
    false,
    DISENGAGED
))]
#[case::disengagement_stop(Case::Feedback(None, Some("stop"), None, false, DISENGAGED))]
#[case::disengagement_myself(Case::Feedback(
    None,
    Some("I'll do it myself"),
    None,
    false,
    DISENGAGED
))]
#[case::disengagement_isnt_working(Case::Feedback(
    None,
    Some("this isn't working"),
    None,
    false,
    DISENGAGED
))]
#[case::disengagement_abort(Case::Feedback(None, Some("abort"), None, false, DISENGAGED))]
#[case::disengagement_talk_to_human(Case::Feedback(
    None,
    Some("talk to human"),
    None,
    false,
    DISENGAGED
))]
#[case::threaded_detectors(Case::Threaded)]
#[case::failure_second_result(Case::Results(
    &[("ok", false), ("boom", true)],
    FAILED
))]
#[case::exhaustion_second_result(Case::Results(
    &[("all fine", false), ("context length exceeded", false)],
    EXHAUSTED
))]
#[case::loop_second_call_matches(Case::Loop(
    &["shell({})", "shell({})"],
    &[("other", "{}"), ("shell", "{}")],
    LOOPED
))]
#[case::satisfaction_capitalized_that_worked(Case::Feedback(
    None,
    Some("That worked"),
    None,
    true,
    SATISFIED
))]
#[case::satisfaction_capitalized_thanks(Case::Feedback(
    None,
    Some("Thanks!"),
    None,
    true,
    SATISFIED
))]
#[case::satisfaction_capitalized_perfect(Case::Feedback(
    None,
    Some("Perfect"),
    None,
    true,
    SATISFIED
))]
#[case::disengagement_capitalized_cancel(Case::Feedback(
    None,
    Some("CANCEL this run"),
    None,
    false,
    DISENGAGED
))]
#[case::disengagement_is_not(Case::Feedback(
    None,
    Some("This is not working"),
    None,
    false,
    DISENGAGED
))]
fn feedback_signals_behaviors(#[case] case: Case) {
    case.run();
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
    let history: Vec<String> = repeat_with(|| "shell({})".to_owned())
        .take(repeats)
        .collect();
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

fn assert_gate(text: &str, phase: GatePhase) {
    let (offset, clean_credit, fires) = match phase {
        GatePhase::Before => (2, false, false),
        GatePhase::At => (1, false, true),
        GatePhase::WithCredit => (1, true, false),
    };
    let mut state = state();
    state.turn_count = i64::from(MIN_TURNS_FOR_CLEAN_CREDIT).saturating_sub(offset);
    state.clean_credit_awarded = clean_credit;
    let turn = make_turn(Some(text), None, Vec::new(), None);
    let delta = apply_turn(&detector(), &mut state, &turn);
    assert_eq!(delta.satisfaction != 0, fires);
}

#[rstest]
#[case::threshold_nan(Case::BoundaryThreshold(
    f64::NAN, 0.5, Some(("misalignment", f64::NAN))
))]
#[case::threshold_infinite(Case::BoundaryThreshold(
    0.45, f64::INFINITY, Some(("stagnation", f64::INFINITY))
))]
#[case::threshold_finite(Case::BoundaryThreshold(0.45, 0.5, None))]
#[case::misalignment_at_threshold(Case::BoundaryMisalignment(
    "a1 a2 a3 a4 a5 a6 a7 a8 a9",
    "a1 a2 a3 a4 a5 a6 a7 a8 a9 b1 b2 b3 b4 b5 b6 b7 b8 b9 b10 b11",
    false,
))]
#[case::misalignment_just_below(Case::BoundaryMisalignment(
    "alpha beta gamma delta epsilon zeta",
    "alpha beta gamma delta eta theta iota",
    true,
))]
#[case::misalignment_just_above(Case::BoundaryMisalignment(
    "p1 p2 p3 p4 p5 q1 q2 q3",
    "p1 p2 p3 p4 p5 r1 r2 r3",
    false,
))]
#[case::misalignment_disjoint(Case::BoundaryMisalignment("hello", "world", false))]
#[case::misalignment_identical(Case::BoundaryMisalignment("hello there", "hello there", false))]
#[case::misalignment_empty_current(Case::BoundaryMisalignment("hello", "", false))]
#[case::stagnation_at_threshold(Case::BoundaryStagnation(
    "red green blue",
    "red green yellow",
    true,
))]
#[case::stagnation_just_below(Case::BoundaryStagnation(
    "alpha beta gamma delta epsilon zeta",
    "alpha beta gamma delta eta theta iota",
    false,
))]
#[case::stagnation_empty(Case::BoundaryStagnation("", "", false))]
#[case::stagnation_identical(Case::BoundaryStagnation("red green blue", "red green blue", true))]
#[case::loop_one_repeat(Case::BoundaryLoop(1, false))]
#[case::loop_two_repeats(Case::BoundaryLoop(2, true))]
#[case::history_at_cap(Case::BoundaryHistory(
    TOOL_CALL_HISTORY_MAX,
    TOOL_CALL_HISTORY_MAX,
    "tool0({})",
))]
#[case::history_over_cap(Case::BoundaryHistory(
    TOOL_CALL_HISTORY_MAX.saturating_add(1),
    TOOL_CALL_HISTORY_MAX,
    "tool1({})",
))]
#[case::history_empty(Case::BoundaryHistory(0, 0, ""))]
#[case::gate_before_threshold(Case::BoundaryGate("thanks", GatePhase::Before))]
#[case::gate_at_threshold(Case::BoundaryGate("thanks", GatePhase::At))]
#[case::gate_clean_credit(Case::BoundaryGate("thanks", GatePhase::WithCredit))]
#[case::status_exhaustion(Case::BoundaryStatus(Some(413), true))]
#[case::status_other(Case::BoundaryStatus(Some(500), false))]
#[case::status_absent(Case::BoundaryStatus(None, false))]
fn feedback_signals_boundaries(#[case] case: Case) {
    case.run();
}
