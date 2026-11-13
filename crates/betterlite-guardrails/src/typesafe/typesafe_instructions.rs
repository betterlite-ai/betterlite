#[must_use]
pub fn question_instructions(question_id: &str) -> String {
    format!(
        "Is tool exchange `{question_id}` in `tool_exchanges` still needed by the assistant to complete `task`? Answer yes if its result contains information the assistant has not yet fully used or will need again; answer no if it is off-topic, superseded, or already incorporated into later messages."
    )
}

#[path = "typesafe_instructions/tests.rs"]
mod tests;
