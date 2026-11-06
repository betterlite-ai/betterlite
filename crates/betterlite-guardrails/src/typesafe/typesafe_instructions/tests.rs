#![cfg(test)]

use super::question_instructions;

#[test]
fn question_instructions_matches_the_litellm_question() {
    assert_eq!(
        question_instructions("e0"),
        "Is tool exchange `e0` in `tool_exchanges` still needed by the assistant to complete `task`? Answer yes if its result contains information the assistant has not yet fully used or will need again; answer no if it is off-topic, superseded, or already incorporated into later messages."
    );
}

#[test]
fn question_instructions_embeds_the_question_id() {
    let instructions = question_instructions("e12");
    assert!(instructions.contains("`e12`"));
    assert_ne!(instructions, question_instructions("e13"));
}
