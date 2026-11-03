#![cfg(test)]
use super::Op;
use crate::helpers::Helper as _;
use serde_json::json;

#[test]
fn identity_helper_serializes_payload_fields() {
    let output = Op::run(&json!({"function":"identity","payload":{"model":"provider/model","metadata":{"user_api_key_team_id":"team"}}})).expect("identity helper succeeds");
    assert_eq!(
        output.get("team_id").and_then(serde_json::Value::as_str),
        Some("team")
    );
    assert_eq!(
        output
            .get("provider_model")
            .and_then(serde_json::Value::as_str),
        Some("provider/model")
    );
}

#[test]
fn baggage_helper_serializes_filtered_identity() {
    let input = json!({"function":"baggage","request_model":"public-model","promoted_keys":["litellm.team.id","gen_ai.request.model","litellm.team.metadata"],"metadata_keys":["requester_metadata.user_id"],"team_metadata_keys":["region"],"identity":{"team_id":"team","team_metadata":{"region":"west","secret":"hidden"},"metadata":{"requester_metadata.user_id":"user","private":"hidden"}}});
    let output = Op::run(&input).expect("baggage helper succeeds");
    assert_eq!(
        output,
        json!({"litellm.team.id":"team","gen_ai.request.model":"public-model","litellm.team.metadata":"{\"region\": \"west\"}","litellm.metadata.user_id":"user"})
    );
}
