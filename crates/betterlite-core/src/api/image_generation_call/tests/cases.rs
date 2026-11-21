#![cfg(test)]

use super::*;
#[test]
fn generation_defaults_and_options() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::OpenAi(String::from(
        "dall-e-2",
    ))));
    let identity = FakeIdentity::failing("resolution should not run without a model");
    let case = Case::new(&ops, &identity, Settings::default());
    let (answer, logging) = case.run(call(None, None));
    let answer = answer.expect("the default dispatch answers");
    assert_eq!(answer.hidden_model.as_deref(), Some("dall-e-2"));
    assert_eq!(
        answer.response.data.as_ref().map(Vec::as_slice),
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
    assert_eq!(sent[0].timeout, Duration::from_secs(600));
    assert_eq!(sent[0].api_base, None);
    assert_eq!(sent[0].now_seconds, NOW);
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

    let ops = FakeOps::new(Some(ImageGenerationConfigTag::OpenAi(String::from(
        "dall-e-3",
    ))));
    let identity = FakeIdentity::new(
        Some("dall-e-3"),
        ProviderIdentity::Known(ProviderId::Openai),
    );
    let case = Case::new(&ops, &identity, Settings::default());
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
    let (answer, _) = case.run(explicit);
    assert!(answer.is_ok());
    assert_eq!(identity.seen().len(), 1);
    let sent = ops.sends();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].organization.as_deref(), Some("org_test"));
    assert_eq!(sent[0].option("n"), Some(&Json::Number(1.into())));
    assert_eq!(
        sent[0].option("quality"),
        Some(&Json::Text(String::from("hd")))
    );
    assert_eq!(
        sent[0].option("response_format"),
        Some(&Json::Text(String::from("b64_json")))
    );
    assert_eq!(
        sent[0].option("size"),
        Some(&Json::Text(String::from("1024x1024")))
    );
    assert_eq!(
        sent[0].option("style"),
        Some(&Json::Text(String::from("natural")))
    );
    assert_eq!(sent[0].option("user"), Some(&Json::Text(String::from("u"))));
    assert_eq!(sent[0].header("X-One"), Some("extra"));
    assert_eq!(sent[0].header("X-Two"), Some("extra"));
    assert_eq!(sent[0].header("X-Keep"), Some("kept"));

    let ops = FakeOps::new(Some(ImageGenerationConfigTag::OpenAi(String::from(
        "dall-e-2",
    ))));
    let identity = FakeIdentity::failing("not reached");
    let case = Case::new(&ops, &identity, Settings::default());
    for mock in ["", "image"] {
        let mut mocked = call(None, None);
        mocked.kwargs = kwargs(vec![("mock_response", Json::Text(mock.to_owned()))]);
        let (answer, _) = case.run(mocked);
        let answer = answer.expect("the mock answers");
        assert_eq!(
            answer
                .response
                .data
                .as_ref()
                .and_then(|data| data[0].url.clone()),
            Some(mock.to_owned())
        );
        assert_eq!(answer.hidden_model.as_deref(), Some("dall-e-2"));
    }
    assert!(ops.sends().is_empty());
}

#[test]
fn azure_image_credentials() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let identity = azure_identity("dep");
    let mut settings = Settings {
        api_base: Some(String::from("https://global.example")),
        api_version: Some(String::from("2024-06-01")),
        api_key: Some(SecretString::from("global-key")),
        azure_key: Some(SecretString::from("azure-key")),
        ..Settings::default()
    };
    settings.azure_api_base = Some(String::from("https://azure-env.example"));
    settings.azure_api_version = Some(String::from("2024-01-01"));
    settings.azure_openai_api_key = Some(SecretString::from("openai-key"));
    settings.azure_api_key = Some(SecretString::from("api-key-env"));
    settings.azure_ad_token = Some(SecretString::from("env-token"));
    let case = Case::new(&ops, &identity, settings);

    let mut base = call(Some("dep"), Some("azure"));
    base.api_base = Some(String::from("https://call.example"));
    base.api_version = Some(String::from("2025-01-01"));
    base.api_key = Some(SecretString::from("call-key"));
    let (answer, _) = case.run(base);
    assert!(answer.is_ok());
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
    let (answer, _) = case.run(fallback);
    assert!(answer.is_ok());
    let sent = ops.sends();
    assert_eq!(sent[1].api_base.as_deref(), Some("https://global.example"));
    assert_eq!(sent[1].api_version.as_deref(), Some("2024-06-01"));
    assert_eq!(sent[1].api_key.as_deref(), Some("global-key"));

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
    let (answer, _) = env_case.run(call(Some("dep"), Some("azure")));
    assert!(answer.is_ok());
    let sent = env_ops.sends();
    assert_eq!(
        sent[0].api_base.as_deref(),
        Some("https://azure-env.example")
    );
    assert_eq!(sent[0].api_version.as_deref(), Some("2024-01-01"));
    assert_eq!(sent[0].api_key.as_deref(), Some("api-key-env"));

    let header_ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let header_identity = azure_identity("dep");
    let header_case = Case::new(&header_ops, &header_identity, Settings::default());
    let mut headers = call(Some("dep"), Some("azure"));
    headers.api_key = Some(SecretString::from("resolved-key"));
    headers.kwargs = kwargs(vec![(
        "headers",
        object(&[("api-key", "caller"), ("Content-Type", "custom")]),
    )]);
    let (answer, _) = header_case.run(headers);
    assert!(answer.is_ok());
    let sent = header_ops.sends();
    assert_eq!(sent[0].header("api-key"), Some("caller"));
    assert_eq!(sent[0].header("Content-Type"), Some("custom"));

    let token_ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let token_identity = azure_identity("dep");
    let token_case = Case::new(
        &token_ops,
        &token_identity,
        Settings {
            azure_ad_token: Some(SecretString::from("env-token")),
            ..Settings::default()
        },
    );
    for (value, expected) in [
        (Json::Text(String::from("token")), Some("token")),
        (Json::Text(String::new()), Some("env-token")),
        (Json::Number(5.into()), Some("env-token")),
    ] {
        let mut tokenized = call(Some("dep"), Some("azure"));
        tokenized.kwargs = kwargs(vec![("azure_ad_token", value)]);
        let (answer, _) = token_case.run(tokenized);
        assert!(answer.is_ok());
        let sent = token_ops.sends();
        let sent = sent.last().expect("the call was sent");
        assert_eq!(sent.azure_ad_token.as_deref(), expected);
        assert!(sent.option("azure_ad_token").is_none());
    }
    assert!(token_ops.sends().iter().all(|sent| sent.api_key.is_none()));

    let callback_ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let callback_identity = azure_identity("dep");
    let mut callback_case = Case::new(&callback_ops, &callback_identity, Settings::default());
    callback_case.azure_ad_token_provider = Some(Arc::new(StaticProvider));
    let (answer, _) = callback_case.run(call(Some("dep"), Some("azure")));
    assert!(answer.is_ok());
    assert!(callback_ops.entra().is_empty());
    assert!(callback_ops.sends()[0].has_entra_provider);

    let entra_ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let entra_identity = azure_identity("dep");
    let entra_case = Case::new(&entra_ops, &entra_identity, Settings::default());
    let mut complete = call(Some("dep"), Some("azure"));
    complete.kwargs = kwargs(vec![
        ("tenant_id", Json::Text(String::from("tenant"))),
        ("client_id", Json::Text(String::from("client"))),
        ("client_secret", Json::Text(String::from("secret"))),
    ]);
    let (answer, logging) = entra_case.run(complete);
    assert!(answer.is_ok());
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
    scoped.kwargs = kwargs(vec![
        ("tenant_id", Json::Text(String::from("tenant"))),
        ("client_id", Json::Text(String::from("client"))),
        ("client_secret", Json::Text(String::from("secret"))),
        ("azure_scope", Json::Text(String::from("scope.custom"))),
    ]);
    let (answer, _) = entra_case.run(scoped);
    assert!(answer.is_ok());
    assert_eq!(entra_ops.entra()[1].3, "scope.custom");

    let partial_ops = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let partial_identity = azure_identity("dep");
    let partial_case = Case::new(&partial_ops, &partial_identity, Settings::default());
    let mut partial = call(Some("dep"), Some("azure"));
    partial.kwargs = kwargs(vec![
        ("tenant_id", Json::Text(String::from("tenant"))),
        ("client_id", Json::Text(String::from("client"))),
    ]);
    let (answer, _) = partial_case.run(partial);
    assert!(answer.is_ok());
    assert!(partial_ops.entra().is_empty());
    assert!(!partial_ops.sends()[0].has_entra_provider);
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
    let (answer, _) = case.run(caller);
    assert!(answer.is_ok());
    let sent = ops.sends();
    assert_eq!(sent[0].header("Authorization"), Some("Bearer caller"));
    assert_eq!(sent[0].header("Content-Type"), Some("application/json"));
    assert_eq!(sent[0].header("X-Trace"), Some("t"));
    assert!(sent[0].azure_ad_token.is_none());
    assert!(ops.foundry().is_empty());

    let mut lowered = call(Some("flux"), Some("azure_ai"));
    lowered.kwargs = kwargs(vec![("headers", object(&[("API-KEY", "caller")]))]);
    let (answer, _) = case.run(lowered);
    assert!(answer.is_ok());
    let sent = ops.sends();
    assert_eq!(sent[1].header("API-KEY"), Some("caller"));
    assert!(sent[1].header("api-key").is_none());
    assert!(ops.foundry().is_empty());

    let mut injected = call(Some("flux"), Some("azure_ai"));
    injected.api_key = Some(SecretString::from("resolved-key"));
    injected.kwargs = kwargs(vec![("extra_headers", object(&[("X-Extra", "one")]))]);
    let (answer, _) = case.run(injected);
    assert!(answer.is_ok());
    let sent = ops.sends();
    let last = sent.last().expect("the foundry call was sent");
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
    let case = Case::new(
        &ops,
        &identity,
        Settings {
            api_base: Some(String::from("https://global.example")),
            ..Settings::default()
        },
    );
    let mut selected = call(Some("image-model"), Some(slug));
    selected.api_base = Some(String::from("https://call.example"));
    let (answer, _) = case.run(selected);
    assert!(answer.is_ok());
    assert_eq!(
        ops.configs(),
        vec![(Some(String::from("image-model")), slug.to_owned())]
    );
    let sent = ops.sends();
    assert_eq!(sent[0].branch, "generic");
    assert_eq!(sent[0].config, Some(tag));
    assert_eq!(sent[0].provider.as_deref(), Some(slug));
    assert_eq!(sent[0].api_base.as_deref(), Some("https://call.example"));
    assert_eq!(
        sent[0].litellm("api_base"),
        Some(&Json::Text(String::from("https://call.example")))
    );

    let global = Case::new(
        &ops,
        &identity,
        Settings {
            api_base: Some(String::from("https://global.example")),
            ..Settings::default()
        },
    );
    let (answer, _) = global.run(call(Some("image-model"), Some(slug)));
    assert!(answer.is_ok());
    let sent = ops.sends();
    assert_eq!(sent[1].api_base.as_deref(), Some("https://global.example"));

    let missing = FakeOps::new(None);
    let missing_case = Case::new(&missing, &identity, Settings::default());
    let (answer, _) = missing_case.run(call(Some("image-model"), Some(slug)));
    match answer {
        Err(ImageGenerationException::Mapped { message, .. }) => {
            assert_eq!(
                message,
                format!("image generation config is not supported for {slug}")
            );
        }
        other => panic!("expected the absent config error, got {other:?}"),
    }
    assert!(missing.sends().is_empty());

    let bfl_ops = FakeOps::new(None);
    let bfl_identity =
        FakeIdentity::new(None, ProviderIdentity::Known(ProviderId::BlackForestLabs));
    let bfl_case = Case::new(&bfl_ops, &bfl_identity, Settings::default());
    let (answer, _) = bfl_case.run(call(Some("flux-pro"), Some("black_forest_labs")));
    let error = answer.expect_err("a missing model is rejected");
    assert_eq!(
        error.message(),
        "Model needs to be set for black_forest_labs"
    );
    assert!(bfl_ops.sends().is_empty());

    let bedrock_ops = FakeOps::new(None);
    let bedrock_identity = FakeIdentity::new(None, ProviderIdentity::Known(ProviderId::Bedrock));
    let bedrock_case = Case::new(&bedrock_ops, &bedrock_identity, Settings::default());
    let (answer, _) = bedrock_case.run(call(Some("model"), Some("bedrock")));
    let error = answer.expect_err("a missing model is rejected");
    assert_eq!(error.message(), "Model needs to be set for bedrock");
    assert!(bedrock_ops.sends().is_empty());

    let configured_ops = FakeOps::new(None);
    let bfl_identity = FakeIdentity::new(
        Some("resolved-flux"),
        ProviderIdentity::Known(ProviderId::BlackForestLabs),
    );
    let bfl_case = Case::new(&configured_ops, &bfl_identity, Settings::default());
    let mut asynchronous = call(Some("flux-pro"), Some("black_forest_labs"));
    asynchronous.kwargs = kwargs(vec![("aimg_generation", Json::Bool(true))]);
    let (answer, _) = bfl_case.run(asynchronous);
    assert!(answer.is_ok());
    let sent = configured_ops.sends();
    assert_eq!(sent[0].branch, "black_forest_labs");
    assert_eq!(sent[0].model.as_deref(), Some("resolved-flux"));
    assert!(sent[0].asynchronous);

    let configured_bedrock = FakeOps::new(None);
    let bedrock_identity = FakeIdentity::new(
        Some("resolved-bedrock"),
        ProviderIdentity::Known(ProviderId::Bedrock),
    );
    let bedrock_case = Case::new(&configured_bedrock, &bedrock_identity, Settings::default());
    let (answer, _) = bedrock_case.run(call(Some("model"), Some("bedrock")));
    assert!(answer.is_ok());
    let sent = configured_bedrock.sends();
    assert_eq!(sent[0].branch, "bedrock");
    assert_eq!(sent[0].model.as_deref(), Some("resolved-bedrock"));
    assert!(!sent[0].asynchronous);
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
    let (answer, _) = case.run(sync_call);
    assert!(answer.is_ok());
    assert_eq!(first.calls(), Vec::new());
    assert_eq!(last.calls(), vec![Some(CustomClientKind::Sync)]);
    assert_eq!(last.seen()[0].provider, "myprov");
    assert_eq!(last.seen()[0].model.as_deref(), Some("custom-model"));
    assert_eq!(last.seen()[0].asynchronous, false);

    let async_handler = Arc::new(FixtureHandler::default());
    let mut async_case = Case::new(&ops, &identity, Settings::default());
    async_case.custom_providers.insert(String::from("myprov"));
    async_case.custom_provider_map = vec![custom_provider("myprov", Some(async_handler.clone()))];
    let mut async_call = call(Some("custom-model"), Some("myprov"));
    async_call.client_kind = Some(CustomClientKind::Asynchronous);
    async_call.kwargs = kwargs(vec![("aimg_generation", Json::Bool(true))]);
    let (answer, _) = async_case.run(async_call);
    assert!(answer.is_ok());
    assert_eq!(
        async_handler.calls(),
        vec![Some(CustomClientKind::Asynchronous)]
    );
    assert!(async_handler.seen()[0].asynchronous);

    let sync_for_async = Arc::new(FixtureHandler::default());
    async_case.custom_provider_map = vec![custom_provider("myprov", Some(sync_for_async.clone()))];
    let mut async_call = call(Some("custom-model"), Some("myprov"));
    async_call.client_kind = Some(CustomClientKind::Sync);
    async_call.kwargs = kwargs(vec![("aimg_generation", Json::Bool(true))]);
    let (answer, _) = async_case.run(async_call);
    assert!(answer.is_ok());
    assert_eq!(sync_for_async.calls(), vec![None]);

    let registered = Arc::new(FixtureHandler::default());
    let mut no_handler = Case::new(&ops, &identity, Settings::default());
    no_handler.custom_providers.insert(String::from("myprov"));
    no_handler.custom_provider_map = vec![custom_provider("myprov", None)];
    let (answer, _) = no_handler.run(call(Some("custom-model"), Some("myprov")));
    match answer {
        Err(ImageGenerationException::UnknownProvider { message }) => {
            assert!(message.contains("Unmapped LLM provider for this endpoint"));
            assert!(message.contains("model=custom-model"));
            assert!(message.contains("custom_llm_provider=myprov"));
        }
        other => panic!("expected the unknown provider error, got {other:?}"),
    }
    assert!(registered.calls().is_empty());

    let failing = FakeOps::failing(None, 429, "slow down");
    let identity = FakeIdentity::new(
        Some("custom-model"),
        ProviderIdentity::Custom(String::from("myprov")),
    );
    let mut failing_case = Case::new(&failing, &identity, Settings::default());
    failing_case.custom_providers.insert(String::from("myprov"));
    failing_case.custom_provider_map = vec![custom_provider(
        "myprov",
        Some(Arc::new(FixtureHandler::default())),
    )];
    let (answer, _) = failing_case.run(call(Some("custom-model"), Some("myprov")));
    match answer {
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
    let (answer, logging) = case.run(explicit);
    let answer = answer.expect("the call answers");
    assert_eq!(answer.hidden_model.as_deref(), Some("dall-e-3"));
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
    let (answer, logging) = case.run(mocked);
    let answer = answer.expect("the mock answers");
    assert_eq!(
        answer
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
    let (answer, _) = case.run(call(Some("missing"), Some("openai")));
    match answer {
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
    extra.kwargs = kwargs(vec![("unknown_option", Json::Text(String::from("x")))]);
    let (answer, _) = case.run(extra);
    match answer {
        Err(ImageGenerationException::Mapped { class, message }) => {
            assert!(matches!(class, ErrorClass::UnsupportedParams(_)));
            assert!(message.starts_with("litellm.UnsupportedParamsError: "));
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
    let (answer, _) = case.run(call(Some("m"), Some("cohere")));
    let answer = answer.expect("the fallthrough answers");
    assert_eq!(answer.response.created, NOW);
    assert_eq!(answer.response.data, Some(Vec::new()));
    assert!(ops.sends().is_empty());
}

#[test]
fn extra_headers_merge_over_headers_before_mapping() {
    let mut generic = crate::api::image_generation_call::call::GenericParams {
        headers: BTreeMap::from([(String::from("X-A"), String::from("one"))]),
        ..crate::api::image_generation_call::call::GenericParams::default()
    };
    generic.extra_headers = Some(BTreeMap::from([
        (String::from("X-A"), String::from("two")),
        (String::from("X-B"), String::from("three")),
    ]));
    assert_eq!(
        generic.merged_headers(),
        BTreeMap::from([
            (String::from("X-A"), String::from("two")),
            (String::from("X-B"), String::from("three")),
        ])
    );
}
