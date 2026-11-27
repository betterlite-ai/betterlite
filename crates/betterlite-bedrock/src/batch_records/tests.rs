#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::disallowed_types,
        clippy::too_many_lines,
        reason = "each case states one LiteLLM behavior as a scenario and compares it through serde_json values"
    )
)]

mod cases;
mod corpus;
mod credentials;
mod support;

use rstest::rstest;

#[rstest]
#[case::chat_batch_record(cases::chat_batch_record)]
#[case::text_completion_to_chat(cases::text_completion_to_chat_case)]
#[case::responses_to_chat(cases::responses_to_chat)]
#[case::embedding_batch_record(cases::embedding_batch_record)]
#[case::alias_and_target_model(cases::alias_and_target_model)]
#[case::default_record_id(cases::default_record_id)]
#[case::jsonl_blank_lines(cases::jsonl_blank_lines)]
#[case::malformed_jsonl_line(cases::malformed_jsonl_line)]
#[case::nova_batch_record(cases::nova_batch_record)]
#[case::upload_signing_handoff(cases::upload_signing_handoff)]
fn batch_records_cases(#[case] check: fn()) {
    check();
}
