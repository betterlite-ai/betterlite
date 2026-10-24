#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::case::{HelperCase, parse_helper_case};

#[test]
fn helper_seeds_select_bedrock_request_policy() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_request_policy"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    let calls: Vec<crate::helpers::HelperCall> = seeds
        .iter()
        .map(|source| {
            parse_helper_case(source).unwrap();
            serde_saphyr::from_str::<HelperCase>(source).unwrap().call
        })
        .collect();
    assert!(
        calls
            .iter()
            .all(|call| call.operation == "bedrock_request_policy")
    );
}

#[test]
fn oracle_cases_project_through_invoke() {
    let cases = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/providers/bedrock-request-policy");
    let mut outputs: Vec<(String, Value)> = fs::read_dir(cases)
        .unwrap()
        .filter_map(|entry| {
            let source = fs::read_to_string(entry.unwrap().path()).unwrap();
            let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
            let input: super::BedrockRequestPolicyInput = (case.call.operation == "bedrock_request_policy")
                .then(|| serde_json::from_value(case.call.input.clone()).unwrap())?;
            Some((case.name, super::invoke(&input).unwrap()))
        })
        .collect();
    outputs.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(outputs, expected_case_outputs());
}

fn expected_case_outputs() -> Vec<(String, Value)> {
    [
        ("bedrock-auth-alias-mapping", r#"[["aws_region_name",null],["keep",1]]"#),
        (
            "bedrock-deployment-auth-merge",
            r#"[["aws_bedrock_runtime_endpoint","https://runtime.example"],["aws_region_name","us-east-1"],["aws_session_token",null],["model","deployment-model"],["temperature",0.2],["aws_access_key_id","AKIA_DEPLOYMENT"],["aws_secret_access_key","deployment-secret"]]"#,
        ),
        (
            "bedrock-identity-before-client-budget",
            r#"{"header_names":["x-amzn-bedrock-request-metadata"],"header_pairs":[["X-Amzn-Bedrock-Request-Metadata","{\"user_api_key_team_id\":\"\",\"user_api_key_alias\":\"team\\u3000alias\\t\\u001c\",\"a\":\"spend\",\"c\":\"3\",\"d\":\"4\",\"e\":\"5\"}"]],"owned":true,"resolved":[["user_api_key_team_id",""],["user_api_key_alias","team\u3000alias\t\u001c"],["User_api_key_y","ok"],["a","caller"],["k00","v"],["k01","v"],["k02","v"],["k03","v"],["k04","v"],["k05","v"],["k06","v"],["k07","v"],["k08","v"],["k09","v"],["z","1"],["c","3"]]}"#,
        ),
        ("bedrock-one-response-iterator", r#"[{"yield":null},"stop","stop","stop"]"#),
        (
            "bedrock-owned-empty-removes-spoofed-header",
            r#"[["Host","bedrock-runtime.us-east-1.amazonaws.com"]]"#,
        ),
        (
            "bedrock-request-metadata-owner",
            r#"{"header_names":["x-amzn-bedrock-request-metadata"],"header_pairs":[],"owned":true,"resolved":null}"#,
        ),
        (
            "bedrock-request-metadata-size-characters",
            r#"[{"class":"BadRequestError","message":"requestMetadata must be an object","status":400},{"class":"BadRequestError","message":"requestMetadata must be an object","status":400},{"class":"BadRequestError","message":"requestMetadata has 17 pairs, Bedrock allows at most 16","status":400},null,{"class":"BadRequestError","message":"requestMetadata value for key \"number\" must be a string","status":400},{"class":"BadRequestError","message":"requestMetadata keys must be 1 to 256 characters long, got 0","status":400},{"class":"BadRequestError","message":"requestMetadata keys must be 1 to 256 characters long, got 257","status":400},{"class":"BadRequestError","message":"requestMetadata value for key \"k\" is longer than 256 characters","status":400},null,null,{"class":"BadRequestError","message":"requestMetadata key \"bad!\" has characters Bedrock does not allow","status":400},{"class":"BadRequestError","message":"requestMetadata value for key \"k\" has characters Bedrock does not allow","status":400},{"class":"BadRequestError","message":"requestMetadata value for key \"k\" has characters Bedrock does not allow","status":400}]"#,
        ),
        (
            "bedrock-unowned-user-metadata",
            r#"[["X-Amzn-Bedrock-Request-Metadata","caller"],["X-OTHER",""],["Accept","application/json"],["x-amzn-bedrock-guardrailIdentifier","guard-id"],["X-Amzn-Bedrock-GuardrailVersion","1"]]"#,
        ),
    ]
    .into_iter()
    .map(|(name, output)| (name.to_owned(), serde_json::from_str(output).unwrap()))
    .collect()
}
