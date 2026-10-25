#![cfg(test)]

use super::*;

#[test]
fn response_round_trips_nulls_numbers_and_unknown_keys() {
    let wire = r#"{"usage":{"future":3,"topicPolicyUnits":null},"assessments":[{"contextualGroundingPolicy":{"filters":[{"threshold":1,"score":0.5,"probe":[true]}]}}],"action":null,"actionReason":"r"}"#;
    let response: Response = serde_json::from_str(wire).unwrap();
    assert_eq!(response.action, None);
    assert_eq!(serde_json::to_string(&response).unwrap(), wire);
}

#[test]
fn records_map_pinned_wire_names() {
    let wire = r#"{"sensitiveInformationPolicy":{"piiEntities":[{"type":"EMAIL","match":"a@b.c"}]},"invocationMetrics":{"guardrailProcessingLatency":7}}"#;
    let assessment: Assessment = serde_json::from_str(wire).unwrap();
    let policy = assessment.sensitive_information_policy.unwrap();
    let pii = &policy.pii_entities.unwrap()[0];
    assert_eq!(
        (pii.type_name.as_deref(), pii.match_text.as_deref()),
        (Some("EMAIL"), Some("a@b.c"))
    );
    let metrics = assessment.invocation_metrics.unwrap();
    assert_eq!(metrics.guardrail_processing_latency, Some(7));
    let request: Request = serde_json::from_str(r#"{"source":"OUTPUT"}"#).unwrap();
    assert_eq!(request.source, Some(Source::Output));
}

#[test]
fn absorb_lets_the_later_null_win() {
    let mut merged: Response = serde_json::from_str(r#"{"outputs":[]}"#).unwrap();
    merged.absorb(&serde_json::from_str(r#"{"outputs":null}"#).unwrap());
    assert_eq!(
        serde_json::to_string(&merged).unwrap(),
        r#"{"outputs":null}"#
    );
    merged.absorb(&serde_json::from_str(r#"{"outputs":[{"text":"m"}]}"#).unwrap());
    assert_eq!(
        serde_json::to_string(&merged).unwrap(),
        r#"{"outputs":[{"text":"m"}]}"#
    );
}
