#![cfg(test)]

use super::{
    INTERNAL_KWARG_PREFIX, Input, is_litellm_owned_kwarg, owned_names, owned_wire_names, run, wire,
    wire_names,
};

#[test]
fn case_1_wire_name_projection() {
    assert_eq!(wire("no-log").wire_name, "no-log");
    assert_eq!(
        wire_names("agentic_loop_state")
            .unwrap()
            .first()
            .map(String::as_str),
        Some("_agentic_loop_depth")
    );
    assert_eq!(owned_wire_names("connection_settings").unwrap().len(), 33);
    let error = owned_wire_names("bad_root").unwrap_err();
    assert!(error.contains("bad_root"));
}

#[test]
fn case_2_owned_name_registry() {
    let names = owned_names();
    assert_eq!(names.first().map(String::as_str), Some("api_key"));
    assert_eq!(
        names.get(33).map(String::as_str),
        Some("custom_llm_provider")
    );
    assert!(names.iter().any(|name| name == "no-log"));
    assert!(names.iter().any(|name| name == "input_cost_per_token"));
}

#[test]
fn case_3_internal_prefix_membership() {
    assert!(is_litellm_owned_kwarg(INTERNAL_KWARG_PREFIX));
    assert!(is_litellm_owned_kwarg("_litellm_fixture"));
    assert!(!is_litellm_owned_kwarg("litellm_fixture"));
    assert!(!is_litellm_owned_kwarg("ordinary_parameter"));
}

#[test]
fn case_4_subset_registries() {
    assert_eq!(
        wire_names("agentic_loop_options").unwrap(),
        vec!["max_agentic_loops"]
    );
    assert_eq!(
        wire_names("bedrock_batch")
            .unwrap()
            .first()
            .map(String::as_str),
        Some("aws_batch_role_arn")
    );
    assert!(is_litellm_owned_kwarg("allowed_openai_params"));
    assert!(is_litellm_owned_kwarg("no-log"));
}

#[test]
fn owned_parameter_names_credential_identity_isolation() {
    let first = owned_names();
    let second = owned_names();
    assert_ne!(first.as_ptr(), second.as_ptr());
    assert_eq!(first, second);
    assert!(first.iter().all(|name| !name.contains("fixture-secret")));
}

#[test]
fn owned_parameter_names_secret_safe_failure() {
    let input = Input::WireNameProjection {
        name: "fixture-secret".to_owned(),
        owner: "bad-root".to_owned(),
        root: None,
    };
    let error = run(&input).unwrap_err();
    assert!(!error.contains("fixture-secret"));
    assert!(error.contains("bad-root"));
}
