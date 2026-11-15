#![cfg(test)]

use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;

use super::initialize_aim;
use crate::third_party_init_a::tests::{exposed, params, params_in_mode};

#[test]
fn aim_takes_the_defaults_of_an_empty_config() {
    let built = initialize_aim(&params(""), None).unwrap();
    assert_eq!(built.name, "");
    assert_eq!(built.api_base, None);
    assert!(built.api_key.is_none());
    assert_eq!(built.inspect_embeddings, None);
    assert_eq!(built.timeout, None);
    assert!(!built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn aim_copies_the_configured_fields() {
    let built = initialize_aim(
        &params_in_mode(
            r#"["pre_call", "post_call"]"#,
            r#",
            "api_base": "https://aim.test",
            "api_key": "aim-key",
            "inspect_embeddings": true,
            "timeout": 4.5,
            "default_on": true
            "#,
        ),
        Some("aim-guard"),
    )
    .unwrap();
    assert_eq!(built.name, "aim-guard");
    assert_eq!(built.api_base.as_deref(), Some("https://aim.test"));
    assert_eq!(exposed(built.api_key.as_ref()), Some("aim-key"));
    assert_eq!(built.inspect_embeddings, Some(true));
    assert_eq!(built.timeout, Some(4.5_f64));
    assert!(built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::Many(vec!["pre_call".to_owned(), "post_call".to_owned()])
    );
}
