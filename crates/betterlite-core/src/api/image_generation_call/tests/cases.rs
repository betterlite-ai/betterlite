#![cfg(test)]

use betterlite_models::catalog::BundledCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use serde_json::Number;

use super::super::plan::CatalogIdentity;
use super::*;

#[test]
fn generation_defaults_and_options() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::OpenAi(String::from(
        "dall-e-2",
    ))));
    let identity = FakeIdentity::failing("resolution should not run without a model");
    let case = Case::new(&ops, &identity, Settings::default());
    let (default_answer, logging) = case.run(call(None, None));
    let answered = default_answer.expect("the default dispatch answers");
    assert_eq!(answered.hidden_model.as_deref(), Some("dall-e-2"));
    assert_eq!(
        answered.response.data.as_ref().map(Vec::as_slice),
        Some(
            [ImageData {
                b64_json: None,
                revised_prompt: None,
                url: Some(String::from("https://example.test/generated.png")),
            }]
            .as_slice()
        )
    );
    assert!(identity.seen().is_empty());
    let sent = ops.sends();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].branch, "openai");
    assert_eq!(sent[0].model.as_deref(), Some("dall-e-2"));
    assert_eq!(sent[0].prompt, "cat");
    assert_eq!(sent[0].timeout, Duration::from_secs(600));
    assert_eq!(sent[0].api_base, None);
    assert_eq!(sent[0].now_seconds, NOW);
    assert_eq!(sent[0].client, None);
    assert_eq!(
        logging.model_call_details.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
    assert_eq!(
        logging.litellm_params.get("azure"),
        Some(&Json::Bool(false))
    );
    assert_eq!(
        logging.model_call_details.get("preset_cache_key"),
        Some(&Json::Null)
    );
    assert_eq!(
        logging.model_call_details.get("stream_response"),
        Some(&Json::Object(IndexMap::new()))
    );

    let explicit_ops = FakeOps::new(Some(ImageGenerationConfigTag::OpenAi(String::from(
        "dall-e-3",
    ))));
    let explicit_identity = FakeIdentity::new(
        Some("dall-e-3"),
        ProviderIdentity::Known(ProviderId::Openai),
    );
    let explicit_case = Case::new(&explicit_ops, &explicit_identity, Settings::default());
    let mut explicit = call(Some("dall-e-3"), Some("openai"));
    explicit.n = Some(1);
    explicit.quality = Some(String::from("hd"));
    explicit.response_format = Some(String::from("b64_json"));
    explicit.size = Some(String::from("1024x1024"));
    explicit.style = Some(String::from("natural"));
    explicit.user = Some(String::from("u"));
    explicit.kwargs = kwargs(vec![
        ("organization", Json::Text(String::from("org_test"))),
        (
            "headers",
            object(&[("X-One", "caller"), ("X-Two", "caller"), ("X-Keep", "kept")]),
        ),
        (
            "extra_headers",
            object(&[("X-One", "extra"), ("X-Two", "extra")]),
        ),
    ]);
    explicit_case.run(explicit).0.unwrap();
    assert_eq!(explicit_identity.seen().len(), 1);
    let explicit_sent = explicit_ops.sends();
    assert_eq!(explicit_sent.len(), 1);
    assert_eq!(explicit_sent[0].organization.as_deref(), Some("org_test"));
    assert_eq!(
        explicit_sent[0].option("n"),
        Some(&Json::Number(Number::from(1_u64)))
    );
    assert_eq!(
        explicit_sent[0].option("quality"),
        Some(&Json::Text(String::from("hd")))
    );
    assert_eq!(
        explicit_sent[0].option("response_format"),
        Some(&Json::Text(String::from("b64_json")))
    );
    assert_eq!(
        explicit_sent[0].option("size"),
        Some(&Json::Text(String::from("1024x1024")))
    );
    assert_eq!(
        explicit_sent[0].option("style"),
        Some(&Json::Text(String::from("natural")))
    );
    assert_eq!(
        explicit_sent[0].option("user"),
        Some(&Json::Text(String::from("u")))
    );
    assert_eq!(explicit_sent[0].header("X-One"), Some("extra"));
    assert_eq!(explicit_sent[0].header("X-Two"), Some("extra"));
    assert_eq!(explicit_sent[0].header("X-Keep"), Some("kept"));

    let mock_ops = FakeOps::new(Some(ImageGenerationConfigTag::OpenAi(String::from(
        "dall-e-2",
    ))));
    let mock_identity = FakeIdentity::failing("not reached");
    let mock_case = Case::new(&mock_ops, &mock_identity, Settings::default());
    assert_mock_answer(&mock_case, "");
    assert_mock_answer(&mock_case, "image");
    assert!(mock_ops.sends().is_empty());
}

// an empty mock_response still answers, and both spellings become the generated url
fn assert_mock_answer(case: &Case<'_>, mock: &str) {
    let mut mocked = call(None, None);
    mocked.kwargs = kwargs(vec![("mock_response", Json::Text(mock.to_owned()))]);
    let answered = case.run(mocked).0.expect("the mock answers");
    assert_eq!(
        answered
            .response
            .data
            .as_ref()
            .and_then(|data| data.first())
            .and_then(|entry| entry.url.clone()),
        Some(mock.to_owned())
    );
    assert_eq!(answered.hidden_model.as_deref(), Some("dall-e-2"));
}

#[test]
fn azure_image_credentials() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let identity = azure_identity("dep");
    let settings = Settings {
        api_base: Some(String::from("https://global.example")),
        api_version: Some(String::from("2024-06-01")),
        api_key: Some(SecretString::from("global-key")),
        azure_key: Some(SecretString::from("azure-key")),
        azure_api_type: String::from("azure"),
        azure_api_base: Some(String::from("https://azure-env.example")),
        azure_api_version: Some(String::from("2024-01-01")),
        azure_openai_api_key: Some(SecretString::from("openai-key")),
        azure_api_key: Some(SecretString::from("api-key-env")),
        azure_ad_token: Some(SecretString::from("env-token")),
        azure_ai_api_base: None,
        azure_ai_api_key: None,
    };
    let case = Case::new(&ops, &identity, settings);

    let mut base = call(Some("dep"), Some("azure"));
    base.api_base = Some(String::from("https://call.example"));
    base.api_version = Some(String::from("2025-01-01"));
    base.api_key = Some(SecretString::from("call-key"));
    case.run(base).0.unwrap();
    let sent = ops.sends();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].api_base.as_deref(), Some("https://call.example"));
    assert_eq!(sent[0].api_version.as_deref(), Some("2025-01-01"));
    assert_eq!(sent[0].api_key.as_deref(), Some("call-key"));
    assert_eq!(sent[0].header("api-key"), Some("call-key"));
    assert_eq!(sent[0].header("Content-Type"), Some("application/json"));

    let mut fallback = call(Some("dep"), Some("azure"));
    fallback.api_base = None;
    fallback.api_version = None;
    fallback.api_key = None;
    case.run(fallback).0.unwrap();
    let fallback_sent = ops.sends();
    assert_eq!(
        fallback_sent[1].api_base.as_deref(),
        Some("https://global.example")
    );
    assert_eq!(fallback_sent[1].api_version.as_deref(), Some("2024-06-01"));
    assert_eq!(fallback_sent[1].api_key.as_deref(), Some("global-key"));

    let env_ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let env_identity = azure_identity("dep");
    let env_settings = Settings::default();
    let env_case = Case::new(
        &env_ops,
        &env_identity,
        Settings {
            azure_api_base: Some(String::from("https://azure-env.example")),
            azure_api_version: Some(String::from("2024-01-01")),
            azure_api_key: Some(SecretString::from("api-key-env")),
            ..env_settings
        },
    );
    env_case.run(call(Some("dep"), Some("azure"))).0.unwrap();
    let env_sent = env_ops.sends();
    assert_eq!(
        env_sent[0].api_base.as_deref(),
        Some("https://azure-env.example")
    );
    assert_eq!(env_sent[0].api_version.as_deref(), Some("2024-01-01"));
    assert_eq!(env_sent[0].api_key.as_deref(), Some("api-key-env"));

    let header_ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let header_identity = azure_identity("dep");
    let header_case = Case::new(&header_ops, &header_identity, Settings::default());
    let mut headers = call(Some("dep"), Some("azure"));
    headers.api_key = Some(SecretString::from("resolved-key"));
    headers.kwargs = kwargs(vec![(
        "headers",
        object(&[("api-key", "caller"), ("Content-Type", "custom")]),
    )]);
    header_case.run(headers).0.unwrap();
    let header_sent = header_ops.sends();
    assert_eq!(header_sent[0].header("api-key"), Some("caller"));
    assert_eq!(header_sent[0].header("Content-Type"), Some("custom"));

    let token_ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let token_identity = azure_identity("dep");
    let token_settings = Settings {
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
    let token_case = Case::new(&token_ops, &token_identity, token_settings);
    assert_ad_token(
        &token_case,
        &token_ops,
        Json::Text(String::from("token")),
        Some("token"),
    );
    assert_ad_token(
        &token_case,
        &token_ops,
        Json::Text(String::new()),
        Some("env-token"),
    );
    assert_ad_token(
        &token_case,
        &token_ops,
        Json::Number(Number::from(5_u64)),
        Some("env-token"),
    );
    assert!(
        token_ops
            .sends()
            .iter()
            .all(|token_sent| token_sent.api_key.is_none())
    );

    let callback_ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let callback_identity = azure_identity("dep");
    let mut callback_case = Case::new(&callback_ops, &callback_identity, Settings::default());
    callback_case.azure_ad_token_provider = Some(Arc::new(StaticProvider));
    callback_case
        .run(call(Some("dep"), Some("azure")))
        .0
        .unwrap();
    assert!(callback_ops.entra().is_empty());
    assert!(callback_ops.sends()[0].has_entra_provider);

    let entra_ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let entra_identity = azure_identity("dep");
    let entra_case = Case::new(&entra_ops, &entra_identity, Settings::default());
    let mut complete = call(Some("dep"), Some("azure"));
    complete.kwargs = entra_kwargs("tenant", Vec::new());
    let (entra_answer, logging) = entra_case.run(complete);
    entra_answer.unwrap();
    assert_eq!(
        entra_ops.entra(),
        vec![(
            String::from("tenant"),
            String::from("client"),
            String::from("secret"),
            String::from("https://cognitiveservices.azure.com/.default"),
        )]
    );
    assert_eq!(
        logging.litellm_params.get("azure"),
        Some(&Json::Bool(false))
    );

    let mut scoped = call(Some("dep"), Some("azure"));
    scoped.kwargs = entra_kwargs(
        "tenant",
        vec![("azure_scope", Json::Text(String::from("scope.custom")))],
    );
    entra_case.run(scoped).0.unwrap();
    assert_eq!(entra_ops.entra()[1].3, "scope.custom");

    let partial_ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let partial_identity = azure_identity("dep");
    let partial_case = Case::new(&partial_ops, &partial_identity, Settings::default());
    let mut partial = call(Some("dep"), Some("azure"));
    partial.kwargs = kwargs(vec![
        ("tenant_id", Json::Text(String::from("tenant"))),
        ("client_id", Json::Text(String::from("client"))),
    ]);
    partial_case.run(partial).0.unwrap();
    assert!(partial_ops.entra().is_empty());
    assert!(!partial_ops.sends()[0].has_entra_provider);
}

// the tenant, client and secret LiteLLM needs before it builds an Entra callback, plus any extra kwargs
fn entra_kwargs(tenant: &str, extra: Vec<(&'static str, Json)>) -> IndexMap<String, Json> {
    let mut pairs = vec![
        ("tenant_id", Json::Text(tenant.to_owned())),
        ("client_id", Json::Text(String::from("client"))),
        ("client_secret", Json::Text(String::from("secret"))),
    ];
    pairs.extend(extra);
    kwargs(pairs)
}

// the option or kwarg wins only as a nonempty string; an empty string and a number fall back to env
fn assert_ad_token(case: &Case<'_>, ops: &FakeOps, value: Json, expected: Option<&str>) {
    let mut tokenized = call(Some("dep"), Some("azure"));
    tokenized.kwargs = kwargs(vec![("azure_ad_token", value)]);
    case.run(tokenized).0.unwrap();
    let sends = ops.sends();
    let sent = sends.last().expect("the call was sent");
    assert_eq!(sent.azure_ad_token.as_deref(), expected);
    assert!(sent.option("azure_ad_token").is_none());
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
    let caller_sent = ops.sends();
    assert_eq!(
        caller_sent[0].header("Authorization"),
        Some("Bearer caller")
    );
    assert_eq!(
        caller_sent[0].header("Content-Type"),
        Some("application/json")
    );
    assert_eq!(caller_sent[0].header("X-Trace"), Some("t"));
    assert!(caller_sent[0].azure_ad_token.is_none());
    assert!(ops.foundry().is_empty());

    let mut lowered = call(Some("flux"), Some("azure_ai"));
    lowered.kwargs = kwargs(vec![("headers", object(&[("API-KEY", "caller")]))]);
    case.run(lowered).0.unwrap();
    let lowered_sent = ops.sends();
    assert_eq!(lowered_sent[1].header("API-KEY"), Some("caller"));
    assert!(lowered_sent[1].header("api-key").is_none());
    assert!(ops.foundry().is_empty());

    let mut injected = call(Some("flux"), Some("azure_ai"));
    injected.api_key = Some(SecretString::from("resolved-key"));
    injected.kwargs = kwargs(vec![("extra_headers", object(&[("X-Extra", "one")]))]);
    case.run(injected).0.unwrap();
    let injected_sent = ops.sends();
    let last = injected_sent.last().expect("the foundry call was sent");
    assert_eq!(last.api_key.as_deref(), Some("resolved-key"));
    assert_eq!(last.header("api-key"), Some("resolved-key"));
    assert_eq!(last.header("Content-Type"), Some("application/json"));
    assert_eq!(last.header("X-Extra"), Some("one"));
    assert!(last.azure_ad_token.is_none());
    let extra = last
        .option("extra_headers")
        .expect("extra headers stay in optional params");
    assert_eq!(extra, &object(&[("X-Extra", "one")]));
    assert_eq!(ops.foundry().len(), 1);
    assert_eq!(ops.foundry()[0].0.as_deref(), Some("resolved-key"));
}

#[rstest]
#[case(
    "recraft",
    ProviderId::Recraft,
    ImageGenerationConfigTag::Recraft(String::from("r"))
)]
#[case(
    "aiml",
    ProviderId::Aiml,
    ImageGenerationConfigTag::Aiml(String::from("r"))
)]
#[case(
    "gemini",
    ProviderId::Gemini,
    ImageGenerationConfigTag::Gemini(String::from("r"))
)]
#[case(
    "fal_ai",
    ProviderId::FalAi,
    ImageGenerationConfigTag::FalAi(String::from("r"))
)]
#[case(
    "stability",
    ProviderId::Stability,
    ImageGenerationConfigTag::Stability(String::from("r"))
)]
#[case(
    "runwayml",
    ProviderId::Runwayml,
    ImageGenerationConfigTag::RunwayMl(String::from("r"))
)]
#[case(
    "vertex_ai",
    ProviderId::VertexAi,
    ImageGenerationConfigTag::VertexAi(String::from("r"))
)]
#[case(
    "openrouter",
    ProviderId::Openrouter,
    ImageGenerationConfigTag::OpenRouter(String::from("r"))
)]
#[case(
    "dashscope",
    ProviderId::Dashscope,
    ImageGenerationConfigTag::DashscopeFamily(String::from("r"))
)]
#[case(
    "qwencloud",
    ProviderId::Qwencloud,
    ImageGenerationConfigTag::DashscopeFamily(String::from("r"))
)]
#[case(
    "qwen_ai_platform",
    ProviderId::QwenAiPlatform,
    ImageGenerationConfigTag::DashscopeFamily(String::from("r"))
)]
#[case("edenai", ProviderId::Edenai, ImageGenerationConfigTag::EdenAi)]
fn generation_provider_selection(
    #[case] slug: &str,
    #[case] provider: ProviderId,
    #[case] tag: ImageGenerationConfigTag,
) {
    let ops = FakeOps::new(Some(tag.clone()));
    let identity = FakeIdentity::new(Some("image-model"), ProviderIdentity::Known(provider));
    let global_settings = Settings {
        api_base: Some(String::from("https://global.example")),
        api_version: None,
        api_key: None,
        azure_key: None,
        azure_api_type: String::from("azure"),
        azure_api_base: None,
        azure_api_version: None,
        azure_openai_api_key: None,
        azure_api_key: None,
        azure_ad_token: None,
        azure_ai_api_base: None,
        azure_ai_api_key: None,
    };
    let case = Case::new(&ops, &identity, global_settings.clone());
    let mut selected = call(Some("image-model"), Some(slug));
    selected.api_base = Some(String::from("https://call.example"));
    case.run(selected).0.unwrap();
    assert_eq!(
        ops.configs(),
        vec![(Some(String::from("image-model")), slug.to_owned())]
    );
    let selected_sent = ops.sends();
    assert_eq!(selected_sent[0].branch, "generic");
    assert_eq!(selected_sent[0].config, Some(tag));
    assert_eq!(selected_sent[0].provider.as_deref(), Some(slug));
    assert_eq!(
        selected_sent[0].api_base.as_deref(),
        Some("https://call.example")
    );
    assert_eq!(
        selected_sent[0].litellm("api_base"),
        Some(&Json::Text(String::from("https://call.example")))
    );

    let global = Case::new(&ops, &identity, global_settings);
    global.run(call(Some("image-model"), Some(slug))).0.unwrap();
    let global_sent = ops.sends();
    assert_eq!(
        global_sent[1].api_base.as_deref(),
        Some("https://global.example")
    );

    let missing = FakeOps::new(None);
    let missing_case = Case::new(&missing, &identity, Settings::default());
    match missing_case.run(call(Some("image-model"), Some(slug))).0 {
        Err(ImageGenerationException::Mapped { message, .. }) => {
            assert_eq!(
                message,
                format!("image generation config is not supported for {slug}")
            );
        }
        other => panic!("expected the absent config error, got {other:?}"),
    }
    assert!(missing.sends().is_empty());

    let unresolved_bfl_ops = FakeOps::new(None);
    let unresolved_bfl =
        FakeIdentity::new(None, ProviderIdentity::Known(ProviderId::BlackForestLabs));
    let unresolved_bfl_case = Case::new(&unresolved_bfl_ops, &unresolved_bfl, Settings::default());
    let bfl_error = unresolved_bfl_case
        .run(call(Some("flux-pro"), Some("black_forest_labs")))
        .0
        .expect_err("a missing model is rejected");
    assert_eq!(
        bfl_error.message(),
        "Model needs to be set for black_forest_labs"
    );
    assert!(unresolved_bfl_ops.sends().is_empty());

    let unresolved_bedrock_ops = FakeOps::new(None);
    let unresolved_bedrock = FakeIdentity::new(None, ProviderIdentity::Known(ProviderId::Bedrock));
    let unresolved_bedrock_case = Case::new(
        &unresolved_bedrock_ops,
        &unresolved_bedrock,
        Settings::default(),
    );
    let bedrock_error = unresolved_bedrock_case
        .run(call(Some("model"), Some("bedrock")))
        .0
        .expect_err("a missing model is rejected");
    assert_eq!(bedrock_error.message(), "Model needs to be set for bedrock");
    assert!(unresolved_bedrock_ops.sends().is_empty());

    let bfl_ops = FakeOps::new(None);
    let resolved_bfl = FakeIdentity::new(
        Some("resolved-flux"),
        ProviderIdentity::Known(ProviderId::BlackForestLabs),
    );
    let bfl_case = Case::new(&bfl_ops, &resolved_bfl, Settings::default());
    let mut asynchronous = call(Some("flux-pro"), Some("black_forest_labs"));
    asynchronous.kwargs = kwargs(vec![("aimg_generation", Json::Bool(true))]);
    bfl_case.run(asynchronous).0.unwrap();
    let bfl_sent = bfl_ops.sends();
    assert_eq!(bfl_sent[0].branch, "black_forest_labs");
    assert_eq!(bfl_sent[0].model.as_deref(), Some("resolved-flux"));
    assert!(bfl_sent[0].asynchronous);

    let bedrock_ops = FakeOps::new(None);
    let resolved_bedrock = FakeIdentity::new(
        Some("resolved-bedrock"),
        ProviderIdentity::Known(ProviderId::Bedrock),
    );
    let bedrock_case = Case::new(&bedrock_ops, &resolved_bedrock, Settings::default());
    bedrock_case
        .run(call(Some("model"), Some("bedrock")))
        .0
        .unwrap();
    let bedrock_sent = bedrock_ops.sends();
    assert_eq!(bedrock_sent[0].branch, "bedrock");
    assert_eq!(bedrock_sent[0].model.as_deref(), Some("resolved-bedrock"));
    assert!(!bedrock_sent[0].asynchronous);
}

fn custom_provider(
    provider: &str,
    handler: Option<Arc<dyn CustomImageHandler>>,
) -> CustomProviderSlot {
    CustomProviderSlot {
        provider: provider.to_owned(),
        handler,
    }
}

#[test]
fn custom_image_handler_selection() {
    let first = Arc::new(FixtureHandler::default());
    let last = Arc::new(FixtureHandler::default());
    let ops = FakeOps::new(None);
    let identity = FakeIdentity::new(
        Some("custom-model"),
        ProviderIdentity::Custom(String::from("myprov")),
    );
    let mut case = Case::new(&ops, &identity, Settings::default());
    case.custom_providers.insert(String::from("myprov"));
    case.custom_provider_map = vec![
        custom_provider("myprov", Some(first.clone())),
        custom_provider("myprov", Some(last.clone())),
    ];
    let mut sync_call = call(Some("custom-model"), Some("myprov"));
    sync_call.client_kind = Some(CustomClientKind::Sync);
    case.run(sync_call).0.unwrap();
    assert_eq!(first.calls(), Vec::new());
    assert_eq!(last.calls(), vec![Some(CustomClientKind::Sync)]);
    assert_eq!(last.seen()[0].provider, "myprov");
    assert_eq!(last.seen()[0].model.as_deref(), Some("custom-model"));
    assert!(!last.seen()[0].asynchronous);

    let async_handler = Arc::new(FixtureHandler::default());
    let mut async_case = Case::new(&ops, &identity, Settings::default());
    async_case.custom_providers.insert(String::from("myprov"));
    async_case.custom_provider_map = vec![custom_provider("myprov", Some(async_handler.clone()))];
    let mut async_call = call(Some("custom-model"), Some("myprov"));
    async_call.client_kind = Some(CustomClientKind::Asynchronous);
    async_call.kwargs = kwargs(vec![("aimg_generation", Json::Bool(true))]);
    async_case.run(async_call).0.unwrap();
    assert_eq!(
        async_handler.calls(),
        vec![Some(CustomClientKind::Asynchronous)]
    );
    assert!(async_handler.seen()[0].asynchronous);

    let sync_for_async = Arc::new(FixtureHandler::default());
    async_case.custom_provider_map = vec![custom_provider("myprov", Some(sync_for_async.clone()))];
    let mut wrong_kind = call(Some("custom-model"), Some("myprov"));
    wrong_kind.client_kind = Some(CustomClientKind::Sync);
    wrong_kind.kwargs = kwargs(vec![("aimg_generation", Json::Bool(true))]);
    async_case.run(wrong_kind).0.unwrap();
    assert_eq!(sync_for_async.calls(), vec![None]);

    let registered = Arc::new(FixtureHandler::default());
    let mut no_handler = Case::new(&ops, &identity, Settings::default());
    no_handler.custom_providers.insert(String::from("myprov"));
    no_handler.custom_provider_map = vec![custom_provider("myprov", None)];
    match no_handler.run(call(Some("custom-model"), Some("myprov"))).0 {
        Err(ImageGenerationException::UnknownProvider { message }) => {
            assert!(message.contains("Unmapped LLM provider for this endpoint"));
            assert!(message.contains("model=custom-model"));
            assert!(message.contains("custom_llm_provider=myprov"));
        }
        other => panic!("expected the unknown provider error, got {other:?}"),
    }
    assert!(registered.calls().is_empty());

    let failing = FakeOps::failing(None, 429, "slow down");
    let failing_identity = FakeIdentity::new(
        Some("custom-model"),
        ProviderIdentity::Custom(String::from("myprov")),
    );
    let mut failing_case = Case::new(&failing, &failing_identity, Settings::default());
    failing_case.custom_providers.insert(String::from("myprov"));
    failing_case.custom_provider_map = vec![custom_provider(
        "myprov",
        Some(Arc::new(FixtureHandler::default())),
    )];
    match failing_case
        .run(call(Some("custom-model"), Some("myprov")))
        .0
    {
        Err(ImageGenerationException::Mapped { class, .. }) => {
            assert_eq!(class.metadata().status_code, 429);
            assert!(matches!(class, ErrorClass::RateLimit(_)));
        }
        other => panic!("expected the mapped provider failure, got {other:?}"),
    }
}

#[derive(Debug, Clone)]
struct HandlerSeen {
    provider: String,
    model: Option<String>,
    asynchronous: bool,
}

struct FixtureHandler {
    calls: Mutex<Vec<Option<CustomClientKind>>>,
    seen: Mutex<Vec<HandlerSeen>>,
}

impl Default for FixtureHandler {
    fn default() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl FixtureHandler {
    fn calls(&self) -> Vec<Option<CustomClientKind>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn seen(&self) -> Vec<HandlerSeen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CustomImageHandler for FixtureHandler {
    fn generate<'handler>(
        &'handler self,
        request: CustomRequest<'handler>,
        _logging: &'handler mut LoggingState,
    ) -> Pin<Box<dyn Future<Output = Result<BranchAnswer, Box<RawException>>> + Send + 'handler>>
    {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.client);
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(HandlerSeen {
                provider: request.provider.to_owned(),
                model: request.model.map(str::to_owned),
                asynchronous: request.asynchronous,
            });
        Box::pin(async move {
            Ok(BranchAnswer {
                response: ImageResponse {
                    created: NOW,
                    data: Some(Vec::new()),
                    usage: Some(ImageUsage::default()),
                },
                hidden_model: None,
            })
        })
    }
}

#[test]
fn hidden_model_and_logging_guards() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::OpenAi(String::from(
        "dall-e-3",
    ))));
    let identity = FakeIdentity::new(
        Some("dall-e-3"),
        ProviderIdentity::Known(ProviderId::Openai),
    );
    let case = Case::new(&ops, &identity, Settings::default());
    let mut explicit = call(Some("dall-e-3"), Some("openai"));
    explicit.user = Some(String::from("u"));
    explicit.api_key = Some(SecretString::from("call-key"));
    let (explicit_answer, logging) = case.run(explicit);
    let answered = explicit_answer.expect("the call answers");
    assert_eq!(answered.hidden_model.as_deref(), Some("dall-e-3"));
    assert_eq!(logging.user.as_deref(), Some("u"));
    assert_eq!(
        logging.model_call_details.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
    assert!(matches!(
        logging.litellm_params.get("timeout"),
        Some(Json::Number(number)) if number.as_f64() == Some(600.0)
    ));
    assert_eq!(
        logging.litellm_params.get("azure"),
        Some(&Json::Bool(false))
    );
}

#[test]
fn mock_response_runs_after_logging() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::OpenAi(String::from(
        "dall-e-2",
    ))));
    let identity = FakeIdentity::failing("not reached");
    let case = Case::new(&ops, &identity, Settings::default());
    let mut mocked = call(None, None);
    mocked.kwargs = kwargs(vec![("mock_response", Json::Text(String::new()))]);
    let (mocked_answer, logging) = case.run(mocked);
    let answered = mocked_answer.expect("the mock answers");
    assert_eq!(
        answered
            .response
            .data
            .as_ref()
            .and_then(|data| data[0].url.clone()),
        Some(String::new())
    );
    assert_eq!(
        logging.model_call_details.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
    assert!(ops.configs().is_empty());
    assert!(ops.sends().is_empty());
}

#[test]
fn resolution_failures_carry_the_wrapped_message() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::OpenAi(String::from(
        "dall-e-3",
    ))));
    let identity = FakeIdentity::failing("unknown model");
    let case = Case::new(&ops, &identity, Settings::default());
    match case.run(call(Some("missing"), Some("openai"))).0 {
        Err(ImageGenerationException::Mapped { message, .. }) => {
            assert!(message.contains("GetLLMProvider Exception - unknown model"));
            assert!(message.contains("original model: missing"));
        }
        other => panic!("expected the provider resolution error, got {other:?}"),
    }
    assert!(ops.sends().is_empty());
}

#[test]
fn unsupported_params_keep_their_prefix() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::Recraft(String::from(
        "recraft-v3",
    ))));
    let identity = FakeIdentity::new(
        Some("recraft-v3"),
        ProviderIdentity::Known(ProviderId::Recraft),
    );
    let case = Case::new(&ops, &identity, Settings::default());
    let mut extra = call(Some("recraft-v3"), Some("recraft"));
    extra.kwargs = kwargs(vec![("imageConfig", Json::Text(String::from("x")))]);
    match case.run(extra).0 {
        Err(ImageGenerationException::Mapped {
            class: ErrorClass::UnsupportedParams(context),
            message,
        }) => {
            assert!(message.starts_with("litellm.UnsupportedParamsError: "));
            assert_eq!(context.message, message);
        }
        other => panic!("expected the unsupported params error, got {other:?}"),
    }
    assert!(ops.sends().is_empty());
}

#[test]
fn fallthrough_answers_an_empty_response() {
    let ops = FakeOps::new(None);
    let identity = FakeIdentity::new(Some("m"), ProviderIdentity::Known(ProviderId::Cohere));
    let case = Case::new(&ops, &identity, Settings::default());
    let answered = case
        .run(call(Some("m"), Some("cohere")))
        .0
        .expect("the fallthrough answers");
    assert_eq!(answered.response.created, NOW);
    assert_eq!(answered.response.data, Some(Vec::new()));
    assert!(ops.sends().is_empty());
}

#[test]
fn extra_headers_merge_over_headers_before_mapping() {
    let generic = GenericParams {
        aimg_generation: false,
        azure_ad_token_provider: false,
        azure_ad_token: None,
        mock_response: None,
        extra_headers: Some(BTreeMap::from([
            (String::from("X-A"), String::from("two")),
            (String::from("X-B"), String::from("three")),
        ])),
        headers: BTreeMap::from([(String::from("X-A"), String::from("one"))]),
        base_model: None,
        organization: None,
        drop_params: None,
        additional_drop_params: Vec::new(),
        credentials: LiteLlmCredentials::default(),
    };
    assert_eq!(
        generic.merged_headers(),
        BTreeMap::from([
            (String::from("X-A"), String::from("two")),
            (String::from("X-B"), String::from("three")),
        ])
    );
}

#[test]
fn generic_kwargs_reach_their_consumers() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let identity = azure_identity("dep");
    let case = Case::new(&ops, &identity, Settings::default());
    let mut entra = call(Some("dep"), Some("azure"));
    entra.kwargs = entra_kwargs(
        "tenant",
        vec![
            ("base_model", Json::Text(String::from("azure/base"))),
            ("azure_username", Json::Text(String::from("user"))),
            ("azure_password", Json::Text(String::from("pass"))),
            ("azure_ad_token_provider", Json::Null),
        ],
    );
    case.run(entra).0.unwrap();
    let mut callback = call(Some("dep"), Some("azure"));
    callback.kwargs = entra_kwargs(
        "tenant",
        vec![(
            "azure_ad_token_provider",
            Json::Text(String::from("callback")),
        )],
    );
    case.run(callback).0.unwrap();
    let mut empty_tenant = call(Some("dep"), Some("azure"));
    empty_tenant.kwargs = entra_kwargs("", Vec::new());
    case.run(empty_tenant).0.unwrap();
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

    let openai_ops = FakeOps::new(Some(ImageGenerationConfigTag::OpenAi(String::from(
        "dall-e-3",
    ))));
    let openai_identity = FakeIdentity::new(
        Some("dall-e-3"),
        ProviderIdentity::Known(ProviderId::Openai),
    );
    let openai_case = Case::new(&openai_ops, &openai_identity, Settings::default());
    openai_case.run(call(Some("dall-e-3"), None)).0.unwrap();
    let seen = openai_identity.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].model.as_deref(), Some("dall-e-3"));
    assert_eq!(seen[0].provider, None);
    assert_eq!(seen[0].api_base, None);
    assert!(!seen[0].has_key);

    let recraft_ops = FakeOps::new(Some(ImageGenerationConfigTag::Recraft(String::from(
        "recraft-v3",
    ))));
    let recraft_identity = FakeIdentity::new(
        Some("recraft-v3"),
        ProviderIdentity::Known(ProviderId::Recraft),
    );
    let recraft_case = Case::new(&recraft_ops, &recraft_identity, Settings::default());
    for dropping in [
        ("drop_params", Json::Bool(true)),
        (
            "additional_drop_params",
            Json::List(vec![Json::Text(String::from("imageConfig"))]),
        ),
    ] {
        let mut dropped = call(Some("recraft-v3"), Some("recraft"));
        dropped.kwargs = kwargs(vec![
            ("imageConfig", Json::Text(String::from("x"))),
            ("tenant_id", Json::Text(String::from("t"))),
            dropping,
        ]);
        recraft_case.run(dropped).0.unwrap();
    }
    let recraft_sent = recraft_ops.sends();
    assert_eq!(recraft_sent.len(), 2);
    assert!(recraft_sent.iter().all(|sent_call| {
        sent_call.option("imageConfig").is_none() && sent_call.option("tenant_id").is_none()
    }));
}

#[test]
fn catalog_identity_requires_a_model() {
    let catalog = BundledCatalog::default().get();
    let names = BTreeSet::new();
    let environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &catalog,
        known_model_names: &names,
        custom_providers: &names,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let identity = CatalogIdentity { context: &context };
    let error = identity
        .resolve(&ImageResolutionRequest {
            model: None,
            custom_llm_provider: Some("openai"),
            api_base: None,
            api_key: None,
        })
        .expect_err("a call without a model is rejected");
    assert_eq!(
        error.to_string(),
        "model parameter is required but was None. Please provide a valid model name."
    );
}
