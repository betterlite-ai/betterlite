#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use super::{AgenticFollowupInput, AgenticKwargValue, build_agentic_followup_kwargs};

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "covers the complete follow-up projection"
)]
fn agentic_followup_merges_patch_and_excludes_explicit_parameters() {
    let request = BTreeMap::from([
        (
            "shared".to_owned(),
            AgenticKwargValue::Text("request".to_owned()),
        ),
        (
            "tools".to_owned(),
            AgenticKwargValue::Text("already sent".to_owned()),
        ),
    ]);
    let patch = BTreeMap::from([
        (
            "shared".to_owned(),
            AgenticKwargValue::Text("patch".to_owned()),
        ),
        ("new".to_owned(), AgenticKwargValue::Boolean(true)),
    ]);
    let request_params = BTreeSet::from(["tools".to_owned()]);
    let fingerprints = ["prior".to_owned()];
    let input = AgenticFollowupInput {
        request_kwargs: &request,
        patch_kwargs: &patch,
        request_params: &request_params,
        depth: 2,
        max_loops: 3,
        fingerprints: &fingerprints,
        fingerprint: "current",
    };
    let result = build_agentic_followup_kwargs(&input).unwrap();
    assert_eq!(
        result.get("shared"),
        Some(&AgenticKwargValue::Text("patch".to_owned()))
    );
    assert_eq!(result.get("new"), Some(&AgenticKwargValue::Boolean(true)));
    assert_eq!(result.get("tools"), None);
    assert_eq!(
        result.get("_agentic_loop_depth"),
        Some(&AgenticKwargValue::Integer(3))
    );
    assert_eq!(
        result.get("max_agentic_loops"),
        Some(&AgenticKwargValue::Integer(3))
    );
    assert_eq!(
        result.get("_agentic_loop_fingerprints"),
        Some(&AgenticKwargValue::Array(vec![
            AgenticKwargValue::Text("prior".to_owned()),
            AgenticKwargValue::Text("current".to_owned()),
        ]))
    );
}
