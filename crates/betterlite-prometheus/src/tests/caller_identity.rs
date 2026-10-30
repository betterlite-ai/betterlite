use crate::caller_identity_logic::{
    resolve_caller_identity_labels, validate_caller_identity_settings,
};
#[test]
fn resolves_modes() {
    let labels = vec![String::from("model"), String::from("api_key_alias")];
    assert_eq!(
        resolve_caller_identity_labels(
            "litellm_llm_api_latency_metric",
            &labels,
            Some("user_email")
        )
        .unwrap(),
        ["model", "user_email"]
    );
    let conflicts = vec![(
        String::from("litellm_llm_api_latency_metric"),
        vec![String::from("api_key_alias")],
    )];
    validate_caller_identity_settings(Some("user_email"), &conflicts).unwrap_err();
}
