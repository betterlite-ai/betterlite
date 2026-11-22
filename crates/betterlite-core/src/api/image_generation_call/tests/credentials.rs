#![cfg(test)]

use serde_json::Number;

use super::*;

#[test]
fn azure_image_credentials() {
    azure_call_and_global_tiers();
    azure_caller_headers();
    azure_ad_tokens();
    azure_entra_callbacks();
}

fn azure_call_and_global_tiers() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let identity = azure_identity("dep");
    let mut settings = azure_environment();
    settings.api_base = Some(String::from("https://global.example"));
    settings.api_version = Some(String::from("2024-06-01"));
    settings.api_key = Some(SecretString::from("global-key"));
    let case = Case::new(&ops, &identity, settings);
    let mut base = call(Some("dep"), Some("azure"));
    base.api_base = Some(String::from("https://call.example"));
    base.api_version = Some(String::from("2025-01-01"));
    base.api_key = Some(SecretString::from("call-key"));
    case.run(base).0.unwrap();
    case.run(call(Some("dep"), Some("azure"))).0.unwrap();
    let mut based = call(Some("dep"), Some("azure"));
    based.kwargs = kwargs(vec![("base_model", Json::Text(String::from("azure/base")))]);
    let answered = case.run(based).0.expect("the based call answers");
    assert_eq!(answered.hidden_model.as_deref(), Some("azure/base"));
    let sent = ops.sends();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[0].api_base.as_deref(), Some("https://call.example"));
    assert_eq!(sent[0].api_version.as_deref(), Some("2025-01-01"));
    assert_eq!(sent[0].api_key.as_deref(), Some("call-key"));
    assert_eq!(sent[0].header("api-key"), Some("call-key"));
    assert_eq!(sent[0].header("Content-Type"), Some("application/json"));
    assert_eq!(sent[1].api_base.as_deref(), Some("https://global.example"));
    assert_eq!(sent[1].api_version.as_deref(), Some("2024-06-01"));
    assert_eq!(sent[1].api_key.as_deref(), Some("global-key"));
    assert_eq!(sent[2].base_model.as_deref(), Some("azure/base"));
}

// each Azure environment key tier is reached once every tier above it is unset
#[rstest]
#[case::azure_key(Some("azure-key"), Some("openai-key"), "azure-key")]
#[case::openai_key(None, Some("openai-key"), "openai-key")]
#[case::api_key(None, None, "api-key-env")]
fn azure_environment_tiers(
    #[case] azure_key: Option<&str>,
    #[case] openai_key: Option<&str>,
    #[case] expected: &str,
) {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let identity = azure_identity("dep");
    let mut settings = azure_environment();
    settings.azure_key = azure_key.map(SecretString::from);
    settings.azure_openai_api_key = openai_key.map(SecretString::from);
    let case = Case::new(&ops, &identity, settings);
    case.run(call(Some("dep"), Some("azure"))).0.unwrap();
    let sent = ops.sends();
    assert_eq!(
        sent[0].api_base.as_deref(),
        Some("https://azure-env.example")
    );
    assert_eq!(sent[0].api_version.as_deref(), Some("2024-01-01"));
    assert_eq!(sent[0].api_key.as_deref(), Some(expected));
}

// every Azure key tier set, so a test can unset the tiers above the one it wants
fn azure_environment() -> Settings {
    Settings {
        api_base: None,
        api_version: None,
        api_key: None,
        azure_key: Some(SecretString::from("azure-key")),
        azure_api_type: String::from("azure"),
        azure_api_base: Some(String::from("https://azure-env.example")),
        azure_api_version: Some(String::from("2024-01-01")),
        azure_openai_api_key: Some(SecretString::from("openai-key")),
        azure_api_key: Some(SecretString::from("api-key-env")),
        azure_ad_token: None,
        azure_ai_api_base: None,
        azure_ai_api_key: None,
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
    let mut mixed = call(Some("dep"), Some("azure"));
    mixed.api_key = Some(SecretString::from("resolved-key"));
    mixed.kwargs = kwargs(vec![(
        "headers",
        object(&[("API-KEY", "caller"), ("content-type", "custom")]),
    )]);
    case.run(mixed).0.unwrap();
    let sent = ops.sends();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].header("api-key"), Some("caller"));
    assert_eq!(sent[0].header("Content-Type"), Some("custom"));
    assert_eq!(sent[1].header("API-KEY"), Some("caller"));
    assert_eq!(sent[1].header("content-type"), Some("custom"));
    assert_eq!(sent[1].header("api-key"), Some("resolved-key"));
    assert_eq!(sent[1].header("Content-Type"), Some("application/json"));
}

// no Azure config lists azure_ad_token, so it folds into extra_body yet still wins for auth and must leave the options
fn azure_ad_tokens() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let identity = azure_identity("dep");
    let settings = Settings {
        api_base: None,
        api_version: None,
        api_key: None,
        azure_key: None,
        azure_api_type: String::from("azure"),
        azure_api_base: None,
        azure_api_version: None,
        azure_openai_api_key: None,
        azure_api_key: None,
        azure_ad_token: Some(SecretString::from("env-token")),
        azure_ai_api_base: None,
        azure_ai_api_key: None,
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
        assert!(sent.option("extra_body").is_none());
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
    assert_eq!(sent.len(), 4);
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

// LiteLLM builds the Entra callback only when all three parts are non-empty strings
#[rstest]
#[case::empty_secret("tenant", "client", "")]
#[case::empty_client("tenant", "", "secret")]
fn entra_credentials_need_every_part(
    #[case] tenant: &str,
    #[case] client: &str,
    #[case] secret: &str,
) {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let identity = azure_identity("dep");
    let case = Case::new(&ops, &identity, Settings::default());
    let mut azure_call = call(Some("dep"), Some("azure"));
    azure_call.kwargs = kwargs(vec![
        ("tenant_id", Json::Text(tenant.to_owned())),
        ("client_id", Json::Text(client.to_owned())),
        ("client_secret", Json::Text(secret.to_owned())),
    ]);
    case.run(azure_call).0.unwrap();
    let sent = ops.sends();
    assert_eq!(sent.len(), 1);
    assert!(!sent[0].has_entra_provider);
    assert!(ops.entra().is_empty());
}

// `litellm_params_dict.get("azure_scope") or <default>` treats an empty scope as unset
#[test]
fn empty_azure_scope_uses_the_default() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let identity = azure_identity("dep");
    let case = Case::new(&ops, &identity, Settings::default());
    let mut azure_call = call(Some("dep"), Some("azure"));
    azure_call.kwargs = entra_kwargs("tenant", vec![("azure_scope", Json::Text(String::new()))]);
    case.run(azure_call).0.unwrap();
    let sent = ops.sends();
    assert!(sent[0].has_entra_provider);
    let entra = ops.entra();
    assert_eq!(entra.len(), 1);
    assert_eq!(entra[0].3, "https://cognitiveservices.azure.com/.default");
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
    assert!(ops.foundry().is_empty());
    let mut based = call(Some("flux"), Some("azure_ai"));
    based.kwargs = kwargs(vec![(
        "base_model",
        Json::Text(String::from("azure-ai/base")),
    )]);
    let answered = case.run(based).0.expect("the based call answers");
    assert_eq!(answered.hidden_model.as_deref(), Some("azure-ai/base"));
    let sent = ops.sends();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[0].header("Authorization"), Some("Bearer caller"));
    assert_eq!(sent[0].header("Content-Type"), Some("application/json"));
    assert_eq!(sent[0].header("X-Trace"), Some("t"));
    assert!(sent[0].azure_ad_token.is_none());
    assert_eq!(sent[1].header("API-KEY"), Some("caller"));
    assert!(sent[1].header("api-key").is_none());
    assert_eq!(sent[2].base_model.as_deref(), Some("azure-ai/base"));
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

// a Null azure_ad_token_provider is no callback, so the triple still builds one; an empty tenant builds none
#[rstest]
#[case::complete("tenant", Json::Null, true)]
#[case::callback("tenant", Json::Text(String::from("callback")), false)]
#[case::empty_tenant("", Json::Null, false)]
fn generic_kwargs_reach_their_consumers(
    #[case] tenant: &str,
    #[case] provider: Json,
    #[case] builds: bool,
) {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let identity = azure_identity("dep");
    let case = Case::new(&ops, &identity, Settings::default());
    let mut azure_call = call(Some("dep"), Some("azure"));
    azure_call.kwargs = entra_kwargs(
        tenant,
        vec![
            ("base_model", Json::Text(String::from("azure/base"))),
            ("azure_username", Json::Text(String::from("user"))),
            ("azure_password", Json::Text(String::from("pass"))),
            ("azure_ad_token_provider", provider),
        ],
    );
    case.run(azure_call).0.unwrap();
    let sent = ops.sends();
    assert_eq!(sent[0].base_model.as_deref(), Some("azure/base"));
    assert_eq!(ops.configs()[0].0.as_deref(), Some("azure/base"));
    // get_optional_params_image_gen asks the mapper for its supported names with base_model or model
    assert_eq!(ops.mapper_models(), vec![String::from("azure/base")]);
    assert_eq!(
        sent[0].litellm("azure_username"),
        Some(&Json::Text(String::from("user")))
    );
    assert_eq!(
        sent[0].litellm("azure_password"),
        Some(&Json::Text(String::from("pass")))
    );
    assert_eq!(sent[0].has_entra_provider, builds);
    assert_eq!(ops.entra().len(), usize::from(builds));
}
