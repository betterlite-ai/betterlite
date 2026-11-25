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
    assert_eq!(params.timeout, Some(17.0_f64));
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
    assert_eq!(params.timeout, Some(600.0_f64));
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
#[case::absent(None)]
#[case::zero(Some(0.0_f64))]
#[case::seconds(Some(8.0_f64))]
#[test]
fn buffered_azure_projects_structured_read_timeout(#[case] read_timeout: Option<f64>) {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mode = ContentMode {
        is_async: false,
        timeout: FineTuningTimeout::Structured(StructuredTimeout {
            read: read_timeout,
            connect: Some(1.0_f64),
            write: Some(2.0_f64),
            pool: Some(3.0_f64),
        }),
    };
    let target = read.plan(&kwargs, mode).azure(None, None);
    assert_eq!(target.params.timeout, read_timeout);
    assert_eq!(target.mode, mode);
}
