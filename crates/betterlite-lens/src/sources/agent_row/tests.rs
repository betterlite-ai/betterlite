#![cfg(test)]

use super::AgentRow;

#[test]
fn decodes_agent_name() {
    let decoded: AgentRow = serde_json::from_str(r#"{"agent_name": "reviewer"}"#).expect("decodes");
    assert_eq!(
        decoded,
        AgentRow {
            agent_name: "reviewer".to_owned(),
        }
    );
}

#[test]
fn rejects_unknown_fields() {
    let error = serde_json::from_str::<AgentRow>(r#"{"agent_name": "reviewer", "extra": 1}"#);
    error.unwrap_err();
}

#[test]
fn rejects_missing_agent_name() {
    let error = serde_json::from_str::<AgentRow>("{}");
    error.unwrap_err();
}

#[test]
fn rejects_non_string_agent_name() {
    let error = serde_json::from_str::<AgentRow>(r#"{"agent_name": 1}"#);
    error.unwrap_err();
}
