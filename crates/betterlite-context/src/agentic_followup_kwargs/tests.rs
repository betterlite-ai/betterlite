#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;

use super::{
    AgenticFollowupInput, AgenticFollowupKwargsError, AgenticKwargValue,
    build_agentic_followup_kwargs,
};

#[rstest]
#[case::request_only(Some(1), None, false, Some(1))]
#[case::patch_only(None, Some(2), false, Some(2))]
#[case::patch_wins(Some(1), Some(2), false, Some(2))]
#[case::excluded_request(Some(1), None, true, None)]
#[case::excluded_patch(None, Some(2), true, None)]
#[case::excluded_both(Some(1), Some(2), true, None)]
fn agentic_followup_precedence(
    #[case] request_value: Option<i64>,
    #[case] patch_value: Option<i64>,
    #[case] excluded: bool,
    #[case] expected: Option<i64>,
) {
    let request = request_value
        .map(|value| ("tools".to_owned(), AgenticKwargValue::Integer(value)))
        .into_iter()
        .collect();
    let patch = patch_value
        .map(|value| ("tools".to_owned(), AgenticKwargValue::Integer(value)))
        .into_iter()
        .collect();
    let params = if excluded {
        BTreeSet::from(["tools".to_owned()])
    } else {
        BTreeSet::new()
    };
    let input = AgenticFollowupInput {
        request_kwargs: &request,
        patch_kwargs: &patch,
        request_params: &params,
        depth: 0,
        max_loops: 3,
        fingerprints: &[],
        fingerprint: "current",
    };
    let result = build_agentic_followup_kwargs(&input).unwrap();
    assert_eq!(
        result.as_ref().get("tools"),
        expected.map(AgenticKwargValue::Integer).as_ref()
    );
    assert_eq!(
        result.as_ref().len(),
        if expected.is_some() { 4 } else { 3 }
    );
}

#[rstest]
#[case::depth("_agentic_loop_depth", AgenticKwargValue::Integer(3))]
#[case::ceiling("max_agentic_loops", AgenticKwargValue::Integer(7))]
#[case::fingerprints("_agentic_loop_fingerprints", AgenticKwargValue::Array(vec![AgenticKwargValue::Text("prior".to_owned()), AgenticKwargValue::Text("current".to_owned())]))]
fn agentic_followup_metadata(#[case] key: &str, #[case] expected: AgenticKwargValue) {
    let request = BTreeMap::from([(key.to_owned(), AgenticKwargValue::Null)]);
    let patch = BTreeMap::from([(key.to_owned(), AgenticKwargValue::Boolean(false))]);
    let params = BTreeSet::from([key.to_owned()]);
    let fingerprints = ["prior".to_owned()];
    let input = AgenticFollowupInput {
        request_kwargs: &request,
        patch_kwargs: &patch,
        request_params: &params,
        depth: 2,
        max_loops: 7,
        fingerprints: &fingerprints,
        fingerprint: "current",
    };
    let result = build_agentic_followup_kwargs(&input).unwrap();
    assert_eq!(result.as_ref().get(key), Some(&expected));
    assert_eq!(result.as_ref().len(), 3);
    assert_eq!(request.get(key), Some(&AgenticKwargValue::Null));
    assert_eq!(patch.get(key), Some(&AgenticKwargValue::Boolean(false)));
    assert_eq!(fingerprints, ["prior"]);
}

#[rstest]
#[case::initial(0, Some(1))]
#[case::maximum(u32::MAX - 1, Some(u32::MAX))]
#[case::overflow(u32::MAX, None)]
fn agentic_followup_depth(#[case] depth: u32, #[case] expected: Option<u32>) {
    let kwargs = BTreeMap::new();
    let params = BTreeSet::new();
    let input = AgenticFollowupInput {
        request_kwargs: &kwargs,
        patch_kwargs: &kwargs,
        request_params: &params,
        depth,
        max_loops: 3,
        fingerprints: &[],
        fingerprint: "current",
    };
    let result = build_agentic_followup_kwargs(&input);
    if let Some(next) = expected {
        let projection = result.unwrap();
        assert_eq!(
            projection.as_ref().get("_agentic_loop_depth"),
            Some(&AgenticKwargValue::Integer(i64::from(next)))
        );
        assert_eq!(
            projection.as_ref().get("_agentic_loop_fingerprints"),
            Some(&AgenticKwargValue::Array(vec![AgenticKwargValue::Text(
                "current".to_owned()
            )]))
        );
    } else {
        assert_eq!(result, Err(AgenticFollowupKwargsError::DepthOverflow));
    }
}

#[test]
fn agentic_followup_snapshot() {
    let nested = AgenticKwargValue::Object(BTreeMap::from([(
        "enabled".to_owned(),
        AgenticKwargValue::Boolean(true),
    )]));
    let mut request = BTreeMap::from([("options".to_owned(), nested.clone())]);
    let patch = BTreeMap::new();
    let params = BTreeSet::new();
    let input = AgenticFollowupInput {
        request_kwargs: &request,
        patch_kwargs: &patch,
        request_params: &params,
        depth: 0,
        max_loops: 3,
        fingerprints: &[],
        fingerprint: "current",
    };
    let result = build_agentic_followup_kwargs(&input).unwrap();
    request.clear();
    assert_eq!(result.as_ref().get("options"), Some(&nested));
    let mut owned_copy = result.as_ref().clone();
    owned_copy.clear();
    assert_eq!(result.as_ref().len(), 4);
}
