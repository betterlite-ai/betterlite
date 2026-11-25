use super::*;

#[tokio::test]
async fn buffered_azure_preserves_trusted_auth_params() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let trusted = TrustedCredentials {
        values: auth_values(),
    };
    let kwargs = ContentKwargs {
        api_key: None,
        api_base: None,
        api_version: None,
        organization: None,
        timeout: None,
        request_timeout: Some(ReadTimeoutValue::Number(Number::from(17_u32))),
        max_retries: Some(4),
        litellm_call_id: None,
        id: None,
    };
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("azure"));
    let invocation = http_context(ClientKind::Sync, Some(&trusted));
    run(&read, ContentEntry::Async, &call, invocation)
        .await
        .unwrap();
    let params = read.dispatch.azure_params.lock().unwrap().pop().unwrap();
    assert_eq!(params.azure_ad_token.unwrap().expose_secret(), "ad-token");
    assert_eq!(params.auth.tenant_id.as_deref(), Some("tenant"));
    assert_eq!(params.auth.client_id.as_deref(), Some("client"));
    assert_eq!(params.auth.client_secret.unwrap().expose_secret(), "secret");
    assert_eq!(params.auth.azure_username.as_deref(), Some("user"));
    assert_eq!(
        params.auth.azure_password.unwrap().expose_secret(),
        "password"
    );
    assert_eq!(params.auth.azure_scope.as_deref(), Some("scope"));
    assert_eq!(params.max_retries, Some(4));
    assert_eq!(params.timeout, None);
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[tokio::test]
async fn buffered_azure_preserves_absent_and_empty_auth(#[case] value: Option<&str>) {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let trusted = TrustedCredentials {
        values: value
            .map(|text| {
                BTreeMap::from([
                    (String::from("azure_ad_token"), SecretString::from(text)),
                    (String::from("tenant_id"), SecretString::from(text)),
                ])
            })
            .unwrap_or_default(),
    };
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("azure"));
    let invocation = http_context(ClientKind::Sync, value.map(|_text| &trusted));
    run(&read, ContentEntry::Sync, &call, invocation)
        .await
        .unwrap();
    let params = read.dispatch.azure_params.lock().unwrap().pop().unwrap();
    assert_eq!(
        params
            .azure_ad_token
            .as_ref()
            .map(SecretString::expose_secret),
        value
    );
    assert_eq!(params.auth.tenant_id.as_deref(), value);
    assert!(params.auth.client_secret.is_none());
    assert!(params.auth.client_id.is_none());
    assert!(params.auth.azure_username.is_none());
    assert!(params.auth.azure_password.is_none());
    assert!(params.auth.azure_scope.is_none());
    assert_eq!(params.max_retries, None);
    assert_eq!(params.timeout, None);
}

fn auth_values() -> BTreeMap<String, SecretString> {
    [
        ("azure_ad_token", "ad-token"),
        ("tenant_id", "tenant"),
        ("client_id", "client"),
        ("client_secret", "secret"),
        ("azure_username", "user"),
        ("azure_password", "password"),
        ("azure_scope", "scope"),
    ]
    .map(|(key, value)| (key.to_owned(), SecretString::from(value)))
    .into_iter()
    .collect()
}

#[rstest]
#[case::absent("null", None)]
#[case::number("12", Some(12.0_f64))]
#[case::zero("0", Some(0.0_f64))]
#[case::text(r#""12.5""#, Some(12.5_f64))]
#[case::empty_text(r#""""#, None)]
#[case::read(r#"{"connect":1,"read":8,"write":2,"pool":3}"#, Some(8.0_f64))]
#[case::zero_read(r#"{"read":0}"#, Some(0.0_f64))]
#[case::absent_read(r#"{"connect":1}"#, None)]
#[case::null_read(r#"{"read":null}"#, None)]
#[tokio::test]
async fn buffered_azure_projects_raw_timeout(#[case] raw: &str, #[case] expected: Option<f64>) {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs {
        api_key: None,
        api_base: None,
        api_version: None,
        organization: None,
        timeout: serde_json::from_str(raw).unwrap(),
        request_timeout: Some(ReadTimeoutValue::Number(Number::from(30_u32))),
        max_retries: None,
        litellm_call_id: None,
        id: None,
    };
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("azure"));
    run(&read, ContentEntry::Async, &call, bare())
        .await
        .unwrap();
    let params = read.dispatch.azure_params.lock().unwrap().pop().unwrap();
    assert_eq!(params.timeout, expected);
}
