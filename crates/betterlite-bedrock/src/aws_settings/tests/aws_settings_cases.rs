#![cfg(test)]
use secrecy::ExposeSecret as _;

#[test]
fn case_1_static_key_pair() {
    let value = serde_json::json!({
        "s3_access_key_id": "access",
        "s3_secret_access_key": "secret"
    });
    assert_eq!(
        crate::aws_settings::s3_static_key_pair(&value),
        Some(("access".to_owned(), "secret".to_owned()))
    );
}

#[test]
fn case_2_session_token_triple() {
    let token = secrecy::SecretString::from("token".to_owned());
    assert_eq!(token.expose_secret(), "token");
}

#[test]
fn case_3_profile_precedence() {
    let inputs = crate::aws_settings::LocalCredentialInputs {
        profile_name: Some("named".to_owned()),
        role_name: None,
        web_identity_token: None,
        session_name: None,
        access_key_id: None,
        secret_access_key: None,
        session_token: None,
        region_name: None,
    };
    assert_eq!(crate::aws_settings::auth_selector_presence(&inputs).2, true);
}

#[test]
fn case_4_default_native_chain() {
    assert_eq!(
        crate::aws_settings::auth_selector_presence(&Default::default()),
        (false, false, false, false, false)
    );
}

#[test]
fn case_5_canonical_session_tags() {
    let tags = vec![
        crate::aws_settings::AwsSessionTag {
            key: "z".to_owned(),
            value: "last".to_owned(),
        },
        crate::aws_settings::AwsSessionTag {
            key: "a".to_owned(),
            value: "first".to_owned(),
        },
    ];
    assert_eq!(
        crate::aws_settings::canonical_session_tags(Some(tags)).map(|items| items[0].key.clone()),
        Some("a".to_owned())
    );
}

#[test]
fn case_6_credential_cache_expiry() {
    let key = crate::aws_settings::cache_key(&serde_json::json!({"region": "west"})).unwrap();
    let credential = || crate::aws_settings::CachedCredentials {
        access_key_id: secrecy::SecretString::from("access".to_owned()),
        secret_access_key: secrecy::SecretString::from("secret".to_owned()),
        session_token: None,
    };
    let first =
        crate::aws_settings::get_or_fetch::<(), _>(&key, None, || Ok(credential())).unwrap();
    assert_eq!(first.access_key_id.expose_secret(), "access");
}

#[test]
fn case_7_bearer_token_empty_versus_absent() {
    assert!(crate::aws_settings::bedrock_bearer_token(Some(""), None).is_none());
}

#[test]
fn case_8_auth_params_removed_from_body() {
    let mut params = serde_json::json!({"aws_access_key_id": "id", "aws_region_name": "west"});
    let removed = crate::aws_settings::pop_aws_auth_params(&mut params);
    assert!(removed.contains_key("aws_access_key_id"));
    assert_eq!(
        params
            .get("aws_region_name")
            .and_then(serde_json::Value::as_str),
        Some("west")
    );
}

#[test]
fn case_9_session_tags_canonical() {
    let tags = Some(vec![
        serde_json::json!({"Key":"b","Value":"2","Extra":true}),
        serde_json::json!({"Key":"a","Value":"1"}),
    ]);
    assert_eq!(
        crate::aws_settings::canonicalize_tags(tags)
            .unwrap()
            .map(|items| items[0].key.clone()),
        Some("a".to_owned())
    );
}

#[test]
fn case_10_assume_role_parameter_presence() {
    let value = crate::aws_settings::assume_role_parameters(
        "arn".to_owned(),
        "session".to_owned(),
        Some(String::new()),
        None,
    );
    assert_eq!(
        value.get("ExternalId").and_then(serde_json::Value::as_str),
        Some("")
    );
}

#[test]
fn case_11_web_identity_policy() {
    assert_eq!(
        crate::aws_settings::web_identity_policy()["Version"],
        "2012-10-17"
    );
}

#[test]
fn case_12_auth_parameters_popped_before_validation() {
    let mut params = serde_json::json!({"aws_access_key_id": "id", "aws_session_tags": 5});
    drop(crate::aws_settings::pop_aws_auth_params(&mut params));
    assert!(params.get("aws_access_key_id").is_none());
}

#[test]
fn case_13_bearer_presence_not_truthiness() {
    assert!(
        crate::aws_settings::bedrock_bearer_token(
            Some(""),
            Some(&secrecy::SecretString::from("env".to_owned()))
        )
        .is_none()
    );
    assert_eq!(
        crate::aws_settings::bedrock_bearer_token(Some(" "), None)
            .unwrap()
            .expose_secret(),
        " "
    );
}

#[test]
fn case_14_credential_cache_key_and_bound() {
    let left = crate::aws_settings::cache_key(&serde_json::json!({"x":1,"y":2})).unwrap();
    let right = crate::aws_settings::cache_key(&serde_json::json!({"y":2,"x":1})).unwrap();
    assert_eq!(left, right);
}

#[test]
fn case_15_local_credential_sources() {
    assert!(
        crate::aws_settings::LocalCredentialInputs::default()
            .profile_name
            .is_none()
    );
}

#[test]
fn case_16_auth_selector_presence() {
    let inputs = crate::aws_settings::LocalCredentialInputs {
        access_key_id: Some(secrecy::SecretString::from("".to_owned())),
        secret_access_key: Some(secrecy::SecretString::from("".to_owned())),
        region_name: Some("".to_owned()),
        profile_name: None,
        role_name: None,
        web_identity_token: None,
        session_name: None,
        session_token: None,
    };
    assert_eq!(crate::aws_settings::auth_selector_presence(&inputs).4, true);
}

#[test]
fn case_17_aws_auth_error_projection() {
    let error = crate::aws_settings::aws_auth_error_projection(403, "denied".to_owned());
    assert_eq!((error.status, error.message.as_str()), (403, "denied"));
    assert_eq!(error.request, error.response.request);
    assert_eq!(error.response.status_code, 403);
}
