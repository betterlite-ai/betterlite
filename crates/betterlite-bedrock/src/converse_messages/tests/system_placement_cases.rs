use betterlite_prompts::arguments::json::Json;

use crate::converse_messages::system::transform_system_message;
use crate::model_routes::ParamMap;

use super::support::{blocks, expected, json_list, messages, render, text_turn};

fn order(history: &str) -> Json {
    let (kept, system) = transform_system_message(&messages(history), None, &ParamMap::new())
        .expect("the messages transform");
    assert_eq!(blocks(&system), expected("[]"), "{history}");
    render(&kept)
}

fn placed(history: &str) -> (Json, Json) {
    let (kept, system) = transform_system_message(&messages(history), None, &ParamMap::new())
        .expect("the messages transform");
    (render(&kept), blocks(&system))
}

const QUESTION: &str = r#"{"role": "user", "content": "question"}"#;
const AGAIN: &str = r#"{"role": "user", "content": "again"}"#;
const NEXT: &str = r#"{"role": "user", "content": "next"}"#;
const REPLY: &str = r#"{"role": "assistant", "content": "reply"}"#;
const CALLING: &str = r#"{"role": "assistant", "content": "calling", "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": "lookup", "arguments": "{}"}}]}"#;
const EMPTY_CALLS: &str = r#"{"role": "assistant", "content": "a", "tool_calls": []}"#;
const R1: &str = r#"{"role": "tool", "tool_call_id": "call_1", "content": "r1"}"#;
const R2: &str = r#"{"role": "tool", "tool_call_id": "call_1", "content": "r2"}"#;
const USER_RESULT: &str =
    r#"{"role": "user", "content": [{"type": "tool_result", "tool_use_id": "c", "content": "r"}]}"#;
const TEXT_THEN_RESULT: &str = r#"{"role": "user", "content": [{"type": "text", "text": "x"}, {"type": "tool_result", "tool_use_id": "c", "content": "r"}]}"#;
const FUNCTION_TURN: &str = r#"{"role": "function", "name": "f", "content": "r"}"#;
const SYSTEM_TOP: &str = r#"{"role": "system", "content": "top"}"#;
const SYSTEM_HOLD: &str = r#"{"role": "system", "content": "hold"}"#;
const SYSTEM_MID: &str = r#"{"role": "system", "content": "mid"}"#;
const SYSTEM_FIRST: &str = r#"{"role": "system", "content": "first"}"#;
const SYSTEM_SECOND: &str = r#"{"role": "system", "content": "second"}"#;

pub(super) fn system_around_tool_result() {
    moves_after_a_tool_run();
    moves_after_a_single_tool_result();
    moves_after_a_user_tool_result();
    moves_after_a_mixed_result_run();
    moves_a_whole_system_run();
    hoists_the_prefix_before_placing();
    leaves_incomplete_adjacency_alone();
    converts_systems_elsewhere_in_place();
}

fn moves_after_a_tool_run() {
    let hold = text_turn("hold");
    let history = json_list(&[QUESTION, CALLING, SYSTEM_HOLD, R1, R2, NEXT]);
    let wanted = format!("[{QUESTION}, {CALLING}, {R1}, {R2}, {hold}, {NEXT}]");
    assert_eq!(order(&history), expected(&wanted));
}

fn moves_after_a_single_tool_result() {
    let hold = text_turn("hold");
    let history = json_list(&[QUESTION, CALLING, SYSTEM_HOLD, R1, NEXT]);
    let wanted = format!("[{QUESTION}, {CALLING}, {R1}, {hold}, {NEXT}]");
    assert_eq!(order(&history), expected(&wanted));
}

fn moves_after_a_user_tool_result() {
    let hold = text_turn("hold");
    let history = json_list(&[QUESTION, CALLING, SYSTEM_HOLD, USER_RESULT, NEXT]);
    let wanted = format!("[{QUESTION}, {CALLING}, {USER_RESULT}, {hold}, {NEXT}]");
    assert_eq!(order(&history), expected(&wanted));
}

fn moves_after_a_mixed_result_run() {
    let history = json_list(&[CALLING, SYSTEM_HOLD, USER_RESULT, R2]);
    let wanted = format!("[{CALLING}, {USER_RESULT}, {R2}, {}]", text_turn("hold"));
    assert_eq!(order(&history), expected(&wanted));
}

fn moves_a_whole_system_run() {
    let history = json_list(&[QUESTION, CALLING, SYSTEM_FIRST, SYSTEM_SECOND, R1]);
    let first = text_turn("first");
    let second = text_turn("second");
    let wanted = format!("[{QUESTION}, {CALLING}, {R1}, {first}, {second}]");
    assert_eq!(order(&history), expected(&wanted));
}

fn hoists_the_prefix_before_placing() {
    let history = json_list(&[SYSTEM_TOP, QUESTION, CALLING, SYSTEM_MID, R1]);
    let (kept, system) = placed(&history);
    let wanted = format!("[{QUESTION}, {CALLING}, {R1}, {}]", text_turn("mid"));
    assert_eq!(kept, expected(&wanted));
    assert_eq!(system, expected(r#"[{"text": "top"}]"#));
}

fn leaves_incomplete_adjacency_alone() {
    an_empty_tool_calls_list_does_not_open_a_run();
    a_result_after_a_text_part_does_not_count();
    a_trailing_system_converts_in_place();
    function_messages_do_not_open_a_run();
    a_plain_user_turn_does_not_open_a_run();
    a_leading_tool_result_does_not_open_a_run();
}

fn an_empty_tool_calls_list_does_not_open_a_run() {
    let history = json_list(&[QUESTION, EMPTY_CALLS, SYSTEM_HOLD, R1]);
    let wanted = format!("[{QUESTION}, {EMPTY_CALLS}, {}, {R1}]", text_turn("hold"));
    assert_eq!(order(&history), expected(&wanted));
}

fn a_result_after_a_text_part_does_not_count() {
    let hold = text_turn("hold");
    let history = json_list(&[QUESTION, CALLING, SYSTEM_HOLD, TEXT_THEN_RESULT]);
    let wanted = format!("[{QUESTION}, {CALLING}, {hold}, {TEXT_THEN_RESULT}]");
    assert_eq!(order(&history), expected(&wanted));
}

fn a_trailing_system_converts_in_place() {
    trailing_after_a_call();
    trailing_after_a_reply();
}

fn trailing_after_a_call() {
    let history = json_list(&[QUESTION, CALLING, SYSTEM_HOLD]);
    let wanted = format!("[{QUESTION}, {CALLING}, {}]", text_turn("hold"));
    assert_eq!(order(&history), expected(&wanted));
}

fn trailing_after_a_reply() {
    let history = json_list(&[REPLY, SYSTEM_HOLD]);
    let wanted = format!("[{REPLY}, {}]", text_turn("hold"));
    assert_eq!(order(&history), expected(&wanted));
}

fn function_messages_do_not_open_a_run() {
    a_function_after_a_system_is_converted();
    a_function_before_a_system_holds_the_move();
}

fn a_function_after_a_system_is_converted() {
    let hold = text_turn("hold");
    let history = json_list(&[QUESTION, CALLING, SYSTEM_HOLD, FUNCTION_TURN]);
    let wanted = format!("[{QUESTION}, {CALLING}, {hold}, {FUNCTION_TURN}]");
    assert_eq!(order(&history), expected(&wanted));
}

fn a_function_before_a_system_holds_the_move() {
    let history = json_list(&[QUESTION, FUNCTION_TURN, SYSTEM_HOLD, R1]);
    let wanted = format!("[{QUESTION}, {FUNCTION_TURN}, {}, {R1}]", text_turn("hold"));
    assert_eq!(order(&history), expected(&wanted));
}

fn a_plain_user_turn_does_not_open_a_run() {
    let history = json_list(&[QUESTION, AGAIN, SYSTEM_HOLD, R1]);
    let wanted = format!("[{QUESTION}, {AGAIN}, {}, {R1}]", text_turn("hold"));
    assert_eq!(order(&history), expected(&wanted));
}

fn a_leading_tool_result_does_not_open_a_run() {
    let history = json_list(&[R1, SYSTEM_HOLD]);
    let wanted = format!("[{R1}, {}]", text_turn("hold"));
    assert_eq!(order(&history), expected(&wanted));
}

fn converts_systems_elsewhere_in_place() {
    let history = json_list(&[QUESTION, SYSTEM_HOLD, REPLY]);
    let wanted = format!("[{QUESTION}, {}, {REPLY}]", text_turn("hold"));
    assert_eq!(order(&history), expected(&wanted));
}
