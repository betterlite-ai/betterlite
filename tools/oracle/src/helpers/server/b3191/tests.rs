#![cfg(test)]

use rstest::rstest;

#[rstest]
#[case::agent_fields_list("agent-fields/agent-fields-list")]
#[case::agent_fields_inheritance("agent-fields/agent-fields-inheritance")]
#[case::agent_fields_no_match("agent-fields/agent-fields-no-match")]
#[case::bedrock_inheritance("helpers/agent-fields/bedrock-inheritance")]
#[case::vertex_inheritance("helpers/agent-fields/vertex-inheritance")]
#[case::azure_no_inheritance("helpers/agent-fields/azure-no-inheritance")]
#[case::field_order("helpers/agent-fields/field-order")]
#[case::directive_removal("helpers/agent-fields/directive-removal")]
#[case::agent_catalog_raw("helpers/agent-fields/agent-catalog-raw")]
fn replays_b3191_against_litellm(#[case] name: &str) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join(format!("cases/{name}.yaml"))).unwrap();
    let case = serde_saphyr::from_str::<crate::case::HelperCase>(&source).unwrap();
    let result = crate::helpers::run_case(&case, false).unwrap();
    assert!(matches!(result, crate::helpers::HelperRun::Match));
}
