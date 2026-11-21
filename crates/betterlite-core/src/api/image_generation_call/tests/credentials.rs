#![cfg(test)]

use serde_json::Number;

use super::*;

#[test]
fn azure_image_credentials() {
    azure_call_and_global_tiers();
    azure_environment_tiers();
    azure_caller_headers();
    azure_ad_tokens();
    azure_entra_callbacks();
}

fn azure_call_and_global_tiers() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let identity = azure_identity("dep");
    let settings = Settings {
        api_base: Some(String::from("https://global.example")),
        api_version: Some(String::from("2024-06-01")),
        api_key: Some(SecretString::from("global-key")),
        azure_key: Some(SecretString::from("azure-key")),
        azure_api_base: Some(String::from("https://azure-env.example")),
        azure_api_version: Some(String::from("2024-01-01")),
        azure_openai_api_key: Some(SecretString::from("openai-key")),
        azure_api_key: Some(SecretString::from("api-key-env")),
        ..Settings::default()
    };
    let case = Case::new(&ops, &identity, settings);
    let mut base = call(Some("dep"), Some("azure"));
    base.api_base = Some(String::from("https://call.example"));
    base.api_version = Some(String::from("2025-01-01"));
    base.api_key = Some(SecretString::from("call-key"));
    case.run(base).0.unwrap();
    case.run(call(Some("dep"), Some("azure"))).0.unwrap();
    let sent = ops.sends();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].api_base.as_deref(), Some("https://call.example"));
    assert_eq!(sent[0].api_version.as_deref(), Some("2025-01-01"));
    assert_eq!(sent[0].api_key.as_deref(), Some("call-key"));
    assert_eq!(sent[0].header("api-key"), Some("call-key"));
    assert_eq!(sent[0].header("Content-Type"), Some("application/json"));
    assert_eq!(sent[1].api_base.as_deref(), Some("https://global.example"));
    assert_eq!(sent[1].api_version.as_deref(), Some("2024-06-01"));
    assert_eq!(sent[1].api_key.as_deref(), Some("global-key"));
}

// each Azure environment tier is reached once every tier above it is unset
fn azure_environment_tiers() {
    let full = Settings {
        azure_key: Some(SecretString::from("azure-key")),
        azure_api_base: Some(String::from("https://azure-env.example")),
        azure_api_version: Some(String::from("2024-01-01")),
        azure_openai_api_key: Some(SecretString::from("openai-key")),
        azure_api_key: Some(SecretString::from("api-key-env")),
        ..Settings::default()
    };
    let without_azure_key = Settings {
        azure_key: None,
        ..full.clone()
    };
    let only_api_key = Settings {
        azure_openai_api_key: None,
        ..without_azure_key.clone()
    };
    for (settings, key) in [
        (full, "azure-key"),
        (without_azure_key, "openai-key"),
        (only_api_key, "api-key-env"),
    ] {
        let ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
        let identity = azure_identity("dep");
        let case = Case::new(&ops, &identity, settings);
        case.run(call(Some("dep"), Some("azure"))).0.unwrap();
        let sent = ops.sends();
        assert_eq!(
            sent[0].api_base.as_deref(),
            Some("https://azure-env.example")
        );
        assert_eq!(sent[0].api_version.as_deref(), Some("2024-01-01"));
        assert_eq!(sent[0].api_key.as_deref(), Some(key));
    }
}

fn azure_caller_headers() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let identity = azure_identity("dep");
    let case = Case::new(&ops, &identity, Settings::default());
    let mut headers = call(Some("dep"), Some("azure"));
    headers.api_key = Some(SecretString::from("resolved-key"));
    headers.kwargs = kwargs(vec![(
        "headers",
        object(&[("api-key", "caller"), ("Content-Type", "custom")]),
    )]);
    case.run(headers).0.unwrap();
    let sent = ops.sends();
    assert_eq!(sent[0].header("api-key"), Some("caller"));
    assert_eq!(sent[0].header("Content-Type"), Some("custom"));
}

fn azure_ad_tokens() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let identity = azure_identity("dep");
    let settings = Settings {
        azure_ad_token: Some(SecretString::from("env-token")),
        ..Settings::default()
    };
    let case = Case::new(&ops, &identity, settings);
    for (value, expected) in [
        (Json::Text(String::from("token")), "token"),
        (Json::Text(String::new()), "env-token"),
        (Json::Number(Number::from(5_u64)), "env-token"),
    ] {
        let mut tokenized = call(Some("dep"), Some("azure"));
        tokenized.kwargs = kwargs(vec![("azure_ad_token", value)]);
        case.run(tokenized).0.unwrap();
        let sends = ops.sends();
        let sent = sends.last().expect("the call was sent");
        assert_eq!(sent.azure_ad_token.as_deref(), Some(expected));
        assert!(sent.option("azure_ad_token").is_none());
        assert!(sent.api_key.is_none());
        assert!(sent.header("api-key").is_none());
    }
}

fn azure_entra_callbacks() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let identity = azure_identity("dep");
    let mut callback_case = Case::new(&ops, &identity, Settings::default());
    callback_case.azure_ad_token_provider = Some(Arc::new(StaticProvider));
    callback_case
        .run(call(Some("dep"), Some("azure")))
        .0
        .unwrap();
    assert!(ops.entra().is_empty());
    let case = Case::new(&ops, &identity, Settings::default());
    let scope = vec![("azure_scope", Json::Text(String::from("scope.custom")))];
    let partial = kwargs(vec![
        ("tenant_id", Json::Text(String::from("tenant"))),
        ("client_id", Json::Text(String::from("client"))),
    ]);
    for entries in [
        entra_kwargs("tenant", Vec::new()),
        entra_kwargs("tenant", scope),
        partial,
    ] {
        let mut entra_call = call(Some("dep"), Some("azure"));
        entra_call.kwargs = entries;
        let (answer, logging) = case.run(entra_call);
        answer.unwrap();
        let azure = logging.litellm_params.get("azure");
        assert_eq!(azure, Some(&Json::Bool(false)));
    }
    assert_entra_records(&ops);
}

fn assert_entra_records(ops: &FakeOps) {
    let entra = ops.entra();
    assert_eq!(entra.len(), 2);
    assert_eq!(
        entra[0],
        (
            String::from("tenant"),
            String::from("client"),
            String::from("secret"),
            String::from("https://cognitiveservices.azure.com/.default"),
        )
    );
    assert_eq!(entra[1].3, "scope.custom");
    let sent = ops.sends();
    assert!(sent[0].has_entra_provider);
    assert!(!sent[3].has_entra_provider);
}

// the tenant, client and secret LiteLLM needs before it builds an Entra callback, plus any extra kwargs
fn entra_kwargs(tenant: &str, extra: Vec<(&'static str, Json)>) -> IndexMap<String, Json> {
    let mut entries = vec![
        ("tenant_id", Json::Text(tenant.to_owned())),
        ("client_id", Json::Text(String::from("client"))),
        ("client_secret", Json::Text(String::from("secret"))),
    ];
    entries.extend(extra);
    kwargs(entries)
}

#[test]
fn foundry_caller_authorization() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::AzureAi(String::from(
        "flux",
    ))));
    let identity = FakeIdentity::new(Some("flux"), ProviderIdentity::Known(ProviderId::AzureAi));
    let case = Case::new(&ops, &identity, Settings::default());
    let mut caller = call(Some("flux"), Some("azure_ai"));
    caller.kwargs = kwargs(vec![(
        "headers",
        object(&[("Authorization", "Bearer caller"), ("X-Trace", "t")]),
    )]);
    case.run(caller).0.unwrap();
    let mut lowered = call(Some("flux"), Some("azure_ai"));
    lowered.kwargs = kwargs(vec![("headers", object(&[("API-KEY", "caller")]))]);
    case.run(lowered).0.unwrap();
    let sent = ops.sends();
    assert_eq!(sent[0].header("Authorization"), Some("Bearer caller"));
    assert_eq!(sent[0].header("Content-Type"), Some("application/json"));
    assert_eq!(sent[0].header("X-Trace"), Some("t"));
    assert!(sent[0].azure_ad_token.is_none());
    assert_eq!(sent[1].header("API-KEY"), Some("caller"));
    assert!(sent[1].header("api-key").is_none());
    assert!(ops.foundry().is_empty());
    foundry_injected_auth();
}

fn foundry_injected_auth() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::AzureAi(String::from(
        "flux",
    ))));
    let identity = FakeIdentity::new(Some("flux"), ProviderIdentity::Known(ProviderId::AzureAi));
    let case = Case::new(&ops, &identity, Settings::default());
    let mut injected = call(Some("flux"), Some("azure_ai"));
    injected.api_key = Some(SecretString::from("resolved-key"));
    injected.kwargs = kwargs(vec![("extra_headers", object(&[("X-Extra", "one")]))]);
    case.run(injected).0.unwrap();
    let sent = ops.sends();
    assert_eq!(sent[0].api_key.as_deref(), Some("resolved-key"));
    assert_eq!(sent[0].header("api-key"), Some("resolved-key"));
    assert_eq!(sent[0].header("Content-Type"), Some("application/json"));
    assert_eq!(sent[0].header("X-Extra"), Some("one"));
    assert!(sent[0].azure_ad_token.is_none());
    assert_eq!(
        sent[0].option("extra_headers"),
        Some(&object(&[("X-Extra", "one")]))
    );
    assert_eq!(ops.foundry().len(), 1);
    assert_eq!(ops.foundry()[0].0.as_deref(), Some("resolved-key"));
}

#[test]
fn generic_kwargs_reach_their_consumers() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let identity = azure_identity("dep");
    let case = Case::new(&ops, &identity, Settings::default());
    let complete = vec![
        ("base_model", Json::Text(String::from("azure/base"))),
        ("azure_username", Json::Text(String::from("user"))),
        ("azure_password", Json::Text(String::from("pass"))),
        ("azure_ad_token_provider", Json::Null),
    ];
    let callback = vec![(
        "azure_ad_token_provider",
        Json::Text(String::from("callback")),
    )];
    for entries in [
        entra_kwargs("tenant", complete),
        entra_kwargs("tenant", callback),
        entra_kwargs("", Vec::new()),
    ] {
        let mut azure_call = call(Some("dep"), Some("azure"));
        azure_call.kwargs = entries;
        case.run(azure_call).0.unwrap();
    }
    let sent = ops.sends();
    assert_eq!(sent[0].base_model.as_deref(), Some("azure/base"));
    assert_eq!(ops.configs()[0].0.as_deref(), Some("azure/base"));
    assert_eq!(
        sent[0].litellm("azure_username"),
        Some(&Json::Text(String::from("user")))
    );
    assert_eq!(
        sent[0].litellm("azure_password"),
        Some(&Json::Text(String::from("pass")))
    );
    assert!(sent[0].has_entra_provider);
    assert!(!sent[1].has_entra_provider);
    assert!(!sent[2].has_entra_provider);
    assert_eq!(ops.entra().len(), 1);
}
