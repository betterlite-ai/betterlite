mod agent_id_alias_path;
mod agent_input_text;
mod agent_signing_and_capability;
mod session_id;

use rstest::rstest;

#[rstest]
#[case::agent_id_alias_path(agent_id_alias_path::agent_id_alias_path)]
#[case::session_id(session_id::session_id)]
#[case::agent_input_text(agent_input_text::agent_input_text)]
#[case::agent_signing_and_capability(agent_signing_and_capability::agent_signing_and_capability)]
fn invoke_agent_cases(#[case] case: fn()) {
    case();
}
