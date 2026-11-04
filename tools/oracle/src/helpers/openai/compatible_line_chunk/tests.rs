#![cfg(test)]

use super::Op;
use crate::helpers::Helper as _;
use serde_json::json;

#[test]
fn helper_runs_the_compatible_line_stream() {
    let input = json!({
        "mode": "async",
        "events": [
            {"kind": "line", "line": "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":0,\"completion_tokens\":0,\"total_tokens\":0}}"},
            {"kind": "line", "line": "data: [DONE]"}
        ]
    });
    assert_eq!(
        Op::decode_and_run(&input).unwrap(),
        json!([
            {"kind":"chunk","value":{"text":"","tool_use":null,"is_finished":false,"finish_reason":"","usage":{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0},"index":0}},
            {"kind":"end"}
        ])
    );
}
