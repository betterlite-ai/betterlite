#![cfg(test)]

use super::{
    INTERNAL_KWARG_PREFIX, OwnedRoot, Owner, is_litellm_owned_kwarg, owned_names, owned_wire_names,
    wire, wire_names,
};

#[rstest::rstest]
#[case::wire_name_projection("wire_name_projection")]
#[case::owned_name_registry("owned_name_registry")]
#[case::internal_prefix_membership("internal_prefix_membership")]
#[case::subset_registries("subset_registries")]
fn owned_parameter_names_cases(#[case] case_name: &str) {
    match case_name {
        "wire_name_projection" => {
            assert_eq!(wire("no-log").wire_name, "no-log");
            assert_eq!(wire_names(Owner::Observability).unwrap().len(), 11);
            assert_eq!(
                owned_wire_names(OwnedRoot::ConnectionSettings)
                    .unwrap()
                    .len(),
                33
            );
        }
        "owned_name_registry" => {
            let names = owned_names().unwrap();
            assert_eq!(names.first().map(String::as_str), Some("api_key"));
            assert!(
                names
                    .iter()
                    .any(|candidate| candidate == "allowed_openai_params")
            );
            assert!(
                names
                    .iter()
                    .any(|candidate| candidate == "regional_endpoint_uplift_multiplier")
            );
        }
        "internal_prefix_membership" => {
            assert!(is_litellm_owned_kwarg(INTERNAL_KWARG_PREFIX).unwrap());
            assert!(is_litellm_owned_kwarg("_litellm_fixture").unwrap());
            assert!(!is_litellm_owned_kwarg("litellm_fixture").unwrap());
            assert!(!is_litellm_owned_kwarg("ordinary_parameter").unwrap());
        }
        "subset_registries" => {
            assert_eq!(
                wire_names(Owner::AgenticLoopOptions).unwrap(),
                vec!["max_agentic_loops"]
            );
            assert_eq!(
                wire_names(Owner::BedrockBatchConnection)
                    .unwrap()
                    .first()
                    .map(String::as_str),
                Some("aws_batch_role_arn")
            );
        }
        _ => (),
    }
}

#[test]
fn owned_parameter_names_credential_identity_isolation() {
    let names = owned_names().unwrap();
    assert!(names.iter().all(|name| !name.contains("fixture-secret")));
    assert!(!is_litellm_owned_kwarg("identity_a_fixture_secret").unwrap());
    assert!(!is_litellm_owned_kwarg("identity_b_fixture_secret").unwrap());
}

#[test]
fn owned_parameter_names_secret_safe_failure() {
    let error = owned_wire_names(OwnedRoot::InvalidLeaf).unwrap_err();
    let debug = format!("{error:?}");
    assert!(!debug.contains("fixture-secret"));
    assert_eq!(
        error.to_string(),
        "InvalidLeaf.value is not a dataclass leaf"
    );
}
