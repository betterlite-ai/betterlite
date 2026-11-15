use rstest::rstest;

use crate::primitives::json_value::JsonValue;

use super::support::{
    HASH_ONE, anthropic_response, chat_response, plan_messages, plan_with, responses_response,
    seeded, setup, value,
};

const USER: &str = r#"{"role":"user","content":"hi"}"#;
const MISSING: &str = "[compresr: hash=nope not found, expired, or not issued for this request]";

fn follow_up(response: &str) -> JsonValue {
    let store = seeded(&[(HASH_ONE, "original one")]);
    plan_messages(&plan_with(&store, response, &setup()).unwrap())
}

fn expected(rest: &str) -> JsonValue {
    value(&format!("[{USER},{rest}]").replace("HASH", HASH_ONE))
}

fn chat_with(content: &str, calls: &str) -> String {
    format!(r#"{{"choices":[{{"message":{{"content":{content},"tool_calls":[{calls}]}}}}]}}"#)
}

fn chat_call(id: &str, hash: &str) -> String {
    format!(
        r#"{{"id":"{id}","type":"function","function":{{"name":"compresr_retrieve","arguments":"{{\"hash\":\"{hash}\"}}"}}}}"#
    )
}

#[test]
fn echoes_only_the_retrieve_call_with_its_result_chat_followup() {
    let rest = r#"{"role":"assistant","content":"thinking","tool_calls":[{"id":"call_1","type":"function","function":{"name":"compresr_retrieve","arguments":"{\"hash\":\"HASH\"}"}}]},{"role":"tool","tool_call_id":"call_1","content":"original one"}"#;
    assert_eq!(follow_up(&chat_response(HASH_ONE)), expected(rest));
}

#[test]
fn every_retrieve_call_gets_a_result_in_order_chat_followup() {
    let calls = format!("{},{}", chat_call("c1", "nope"), chat_call("c2", HASH_ONE));
    let rest = format!(
        r#"{{"role":"assistant","content":"t","tool_calls":[{},{}]}},{{"role":"tool","tool_call_id":"c1","content":"{MISSING}"}},{{"role":"tool","tool_call_id":"c2","content":"original one"}}"#,
        chat_call("c1", "nope"),
        chat_call("c2", "HASH")
    );
    assert_eq!(follow_up(&chat_with(r#""t""#, &calls)), expected(&rest));
}

#[rstest]
#[case::text(r#""thinking""#, r#""thinking""#)]
#[case::text_parts(
    r#"[{"type":"text","text":"a"},{"type":"image_url"},{"type":"text","text":"b"}]"#,
    r#""a\n\nb""#
)]
#[case::null("null", "null")]
#[case::empty(r#""""#, "null")]
fn assistant_text_is_rebuilt_from_the_response_chat_followup(
    #[case] content: &str,
    #[case] rebuilt: &str,
) {
    let response = chat_with(content, &chat_call("call_1", HASH_ONE));
    let rest = format!(
        r#"{{"role":"assistant","content":{rebuilt},"tool_calls":[{}]}},{{"role":"tool","tool_call_id":"call_1","content":"original one"}}"#,
        chat_call("call_1", "HASH")
    );
    assert_eq!(follow_up(&response), expected(&rest));
}

#[test]
fn null_sections_leave_the_chat_shape_chat_followup() {
    let response = chat_response(HASH_ONE).replacen('{', r#"{"output":null,"content":null,"#, 1);
    let rest = r#"{"role":"assistant","content":"thinking","tool_calls":[{"id":"call_1","type":"function","function":{"name":"compresr_retrieve","arguments":"{\"hash\":\"HASH\"}"}}]},{"role":"tool","tool_call_id":"call_1","content":"original one"}"#;
    assert_eq!(follow_up(&response), expected(rest));
}

#[test]
fn pairs_tool_use_with_tool_result_anthropic_followup() {
    let rest = r#"{"role":"assistant","content":[{"type":"text","text":"let me look"},{"type":"tool_use","id":"toolu_1","name":"compresr_retrieve","input":{"hash":"HASH"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"original one"}]}"#;
    assert_eq!(follow_up(&anthropic_response(HASH_ONE)), expected(rest));
}

#[rstest]
#[case::no_text_block(
    r#"[{"type":"tool_use","id":"t1","name":"compresr_retrieve","input":{"hash":"HASH"}}]"#,
    ""
)]
#[case::empty_text_skipped(r#"[{"type":"text","text":""},{"type":"tool_use","id":"t1","name":"compresr_retrieve","input":{"hash":"HASH"}}]"#, "")]
#[case::text_blocks_joined(r#"[{"type":"text","text":"a"},{"type":"text","text":"b"},{"type":"tool_use","id":"t1","name":"compresr_retrieve","input":{"hash":"HASH"}}]"#, r#"{"type":"text","text":"ab"},"#)]
fn assistant_text_leads_the_tool_use_blocks_anthropic_followup(
    #[case] content: &str,
    #[case] text_block: &str,
) {
    let response = format!(r#"{{"content":{content}}}"#).replace("HASH", HASH_ONE);
    let rest = format!(
        r#"{{"role":"assistant","content":[{text_block}{{"type":"tool_use","id":"t1","name":"compresr_retrieve","input":{{"hash":"HASH"}}}}]}},{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t1","content":"original one"}}]}}"#
    );
    assert_eq!(follow_up(&response), expected(&rest));
}

#[test]
fn missing_hash_still_gets_a_tool_result_anthropic_followup() {
    let response = r#"{"content":[{"type":"tool_use","id":"t1","name":"compresr_retrieve","input":{"hash":"nope"}},{"type":"tool_use","id":"t2","name":"compresr_retrieve","input":{"hash":"HASH"}}]}"#
        .replace("HASH", HASH_ONE);
    let rest = format!(
        r#"{{"role":"assistant","content":[{{"type":"tool_use","id":"t1","name":"compresr_retrieve","input":{{"hash":"nope"}}}},{{"type":"tool_use","id":"t2","name":"compresr_retrieve","input":{{"hash":"HASH"}}}}]}},{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t1","content":"{MISSING}"}},{{"type":"tool_result","tool_use_id":"t2","content":"original one"}}]}}"#
    );
    assert_eq!(follow_up(&response), expected(&rest));
}

#[test]
fn pairs_function_call_with_its_output_responses_followup() {
    let rest = r#"{"role":"assistant","content":"checking"},{"type":"function_call","call_id":"call_1","name":"compresr_retrieve","arguments":"{\"hash\":\"HASH\"}"},{"type":"function_call_output","call_id":"call_1","output":"original one"}"#;
    assert_eq!(follow_up(&responses_response(HASH_ONE)), expected(rest));
}

#[rstest]
#[case::no_message_item(r#"[{"type":"function_call","call_id":"c1","name":"compresr_retrieve","arguments":"{\"hash\":\"HASH\"}"}]"#, "")]
#[case::chunks_joined(r#"[{"type":"message","content":[{"type":"output_text","text":"a"},{"type":"output_text","text":""},{"type":"output_text","text":"b"}]},{"type":"function_call","call_id":"c1","name":"compresr_retrieve","arguments":"{\"hash\":\"HASH\"}"}]"#, r#"{"role":"assistant","content":"ab"},"#)]
fn assistant_text_leads_the_function_calls_responses_followup(
    #[case] output: &str,
    #[case] assistant: &str,
) {
    let response = format!(r#"{{"output":{output}}}"#).replace("HASH", HASH_ONE);
    let rest = format!(
        r#"{assistant}{{"type":"function_call","call_id":"c1","name":"compresr_retrieve","arguments":"{{\"hash\":\"HASH\"}}"}},{{"type":"function_call_output","call_id":"c1","output":"original one"}}"#
    );
    assert_eq!(follow_up(&response), expected(&rest));
}

#[test]
fn responses_shape_wins_over_the_anthropic_shape_responses_followup() {
    let response = responses_response(HASH_ONE).replacen('{', r#"{"content":[],"#, 1);
    let rest = r#"{"role":"assistant","content":"checking"},{"type":"function_call","call_id":"call_1","name":"compresr_retrieve","arguments":"{\"hash\":\"HASH\"}"},{"type":"function_call_output","call_id":"call_1","output":"original one"}"#;
    assert_eq!(follow_up(&response), expected(rest));
}
