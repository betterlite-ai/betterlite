#![cfg(test)]

use crate::auth_prefetch::rows::{RowKind, validate_row};

use super::support::{Logs, json};

// a rejected row names its column and model, and never echoes what it held
#[test]
fn prefetch_validation_redacts_values() {
    let (logs, _guard) = Logs::capture();
    let rejected = json(
        r#"{"team_id": 42, "members": ["sk-secret-value"], "metadata": {"api_key": "sk-secret-value"}}"#,
    );
    let object = validate_row(RowKind::Team, Some(&rejected), 1234.5);
    assert!(object.is_none(), "the row validates as a team");
    let text = logs.text();
    assert!(
        text.contains("auth prefetch: the row did not validate as its model"),
        "no rejection line: {text}"
    );
    assert!(text.contains(r#"row="team_row""#), "no row label: {text}");
    assert!(
        text.contains(r#"model="LiteLLM_TeamTableCachedObj""#),
        "no model name: {text}"
    );
    assert!(text.contains("category=Data"), "no category: {text}");
    assert!(
        !text.contains("sk-secret-value"),
        "the rejected values reached the log: {text}"
    );
}
