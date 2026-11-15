#![cfg(test)]

use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use rstest::rstest;

use super::initialize_lasso_folder;
use crate::third_party_init_a::error::ThirdPartyInitAError;
use crate::third_party_init_a::tests::{exposed, params, params_in_mode};

#[test]
fn lasso_takes_the_defaults_of_an_empty_config() {
    let built = initialize_lasso_folder(&params(""), None).unwrap();
    assert_eq!(built.name, "");
    assert!(built.api_key.is_none());
    assert_eq!(built.api_base, None);
    assert_eq!(built.user_id, None);
    assert_eq!(built.conversation_id, None);
    assert_eq!(built.timeout, None);
    assert!(!built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn lasso_copies_the_configured_fields() {
    let built = initialize_lasso_folder(
        &params_in_mode(
            r#"["during_call"]"#,
            r#",
            "api_key": "lasso-key",
            "api_base": "https://lasso.test",
            "lasso_user_id": "user-1",
            "lasso_conversation_id": "conversation-2",
            "timeout": 2.5,
            "default_on": true
            "#,
        ),
        Some("lasso-guard"),
    )
    .unwrap();
    assert_eq!(built.name, "lasso-guard");
    assert_eq!(exposed(built.api_key.as_ref()), Some("lasso-key"));
    assert_eq!(built.api_base.as_deref(), Some("https://lasso.test"));
    assert_eq!(built.user_id.as_deref(), Some("user-1"));
    assert_eq!(built.conversation_id.as_deref(), Some("conversation-2"));
    assert_eq!(built.timeout, Some(2.5_f64));
    assert!(built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::Many(vec!["during_call".to_owned()])
    );
}

#[test]
fn lasso_reads_a_null_id_as_unset() {
    let built = initialize_lasso_folder(
        &params(r#", "lasso_user_id": null, "lasso_conversation_id": null"#),
        None,
    )
    .unwrap();
    assert_eq!(built.user_id, None);
    assert_eq!(built.conversation_id, None);
}

#[rstest]
#[case::user_id(r#", "lasso_user_id": 7"#, "lasso_user_id")]
#[case::conversation_id(r#", "lasso_conversation_id": true"#, "lasso_conversation_id")]
fn lasso_names_the_param_of_a_wrong_type(#[case] extras: &str, #[case] field: &str) {
    let error = initialize_lasso_folder(&params(extras), None).unwrap_err();
    assert!(matches!(
        &error,
        ThirdPartyInitAError::InvalidParam { name } if name == field
    ));
}
