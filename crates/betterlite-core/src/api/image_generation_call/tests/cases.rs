#![cfg(test)]

use betterlite_error::envelope::WireField;
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
    default_dispatch();
    explicit_options();
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::OpenAi(String::from(
        "dall-e-2",
    ))));
    let identity = FakeIdentity::failing("not reached");
    let case = Case::new(&ops, &identity, Settings::default());
    assert_mock_answer(&case, "");
    assert_mock_answer(&case, "image");
    assert!(ops.sends().is_empty());
}

fn default_dispatch() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::OpenAi(String::from(
        "dall-e-2",
    ))));
    let identity = FakeIdentity::failing("resolution should not run without a model");
    let case = Case::new(&ops, &identity, Settings::default());
    let (answer, logging) = case.run(call(None, None));
    let answered = answer.expect("the default dispatch answers");
    assert_eq!(answered.hidden_model.as_deref(), Some("dall-e-2"));
    assert_eq!(
        answered.response.data,
        Some(vec![ImageData {
            b64_json: None,
            revised_prompt: None,
            url: Some(String::from("https://example.test/generated.png")),
        }])
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
    assert_default_logging(&logging);
}

fn assert_default_logging(logging: &LoggingState) {
    let provider = logging.model_call_details.get("custom_llm_provider");
    assert_eq!(provider, Some(&Json::Text(String::from("openai"))));
    assert_eq!(
        logging.litellm_params.get("azure"),
        Some(&Json::Bool(false))
    );
    assert_eq!(
        logging.litellm_params.get("preset_cache_key"),
        Some(&Json::Null)
    );
    assert_eq!(
        logging.litellm_params.get("stream_response"),
        Some(&Json::Object(IndexMap::new()))
    );
}

fn explicit_options() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::OpenAi(String::from(
        "dall-e-3",
    ))));
    let identity = FakeIdentity::new(
        Some("dall-e-3"),
        ProviderIdentity::Known(ProviderId::Openai),
    );
    let case = Case::new(&ops, &identity, Settings::default());
    case.run(explicit_call()).0.unwrap();
    assert_eq!(identity.seen().len(), 1);
    let sent = ops.sends();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].organization.as_deref(), Some("org_test"));
    assert_eq!(
        sent[0].option("n"),
        Some(&Json::Number(Number::from(1_u64)))
    );
    for (name, value) in [
        ("quality", "hd"),
        ("response_format", "b64_json"),
        ("size", "1024x1024"),
        ("style", "natural"),
        ("user", "u"),
    ] {
        assert_eq!(sent[0].option(name), Some(&Json::Text(value.to_owned())));
    }
    assert_eq!(sent[0].header("X-One"), Some("extra"));
    assert_eq!(sent[0].header("X-Two"), Some("extra"));
    assert_eq!(sent[0].header("X-Keep"), Some("kept"));
    let extra = object(&[("X-One", "extra"), ("X-Two", "extra")]);
    assert_eq!(sent[0].option("extra_headers"), Some(&extra));
    assert_eq!(
        sent[0].option("extra_body"),
        Some(&Json::Object(IndexMap::from([(
            String::from("extra_headers"),
            extra,
        )])))
    );
    // the OpenAI-only names stay out of the provider body and the organization reaches its own header
    assert!(sent[0].option("organization").is_none());
}

fn explicit_call() -> ImageGenerationCall {
    let mut explicit = call(Some("dall-e-3"), Some("openai"));
    explicit.image_count = Some(1);
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
    explicit
}

// an empty mock_response still answers, both spellings become the generated url, and no hidden model is recorded
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
    assert_eq!(answered.hidden_model, None);
}

// a bare ValueError reaches dispatch as an API connection failure with a 500 and no wire fields
fn assert_plain_value_error(class: &ErrorClass) {
    let metadata = class.metadata();
    assert!(matches!(class, ErrorClass::ApiConnection(_)));
    assert_eq!(metadata.status_code, 500);
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.code, "500");
    assert_eq!(metadata.openai_code, None);
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
    let settings = Settings {
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
    let case = Case::new(&ops, &identity, settings);
    let mut selected = call(Some("image-model"), Some(slug));
    selected.api_base = Some(String::from("https://call.example"));
    case.run(selected).0.unwrap();
    case.run(call(Some("image-model"), Some(slug))).0.unwrap();
    let model = Some(String::from("image-model"));
    assert_eq!(
        ops.configs(),
        vec![(model.clone(), slug.to_owned()), (model, slug.to_owned())]
    );
    let sent = ops.sends();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].branch, "generic");
    assert_eq!(sent[0].config, Some(tag));
    assert_eq!(sent[0].provider.as_deref(), Some(slug));
    assert_eq!(sent[0].api_base.as_deref(), Some("https://call.example"));
    let call_base = Json::Text(String::from("https://call.example"));
    assert_eq!(sent[0].litellm("api_base"), Some(&call_base));
    assert_eq!(sent[1].api_base.as_deref(), Some("https://global.example"));
    let missing = FakeOps::new(None);
    let missing_case = Case::new(&missing, &identity, Settings::default());
    match missing_case.run(call(Some("image-model"), Some(slug))).0 {
        Err(ImageGenerationException::Mapped { class, message }) => {
            assert_eq!(
                message,
                format!("image generation config is not supported for {slug}")
            );
            assert_plain_value_error(&class);
        }
        other => panic!("expected the absent config error, got {other:?}"),
    }
    assert!(missing.sends().is_empty());
}

// LiteLLM's openai branch also takes every openai_compatible_providers entry, not only openai and litellm_proxy
#[test]
fn openai_compatible_providers_take_the_openai_branch() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::OpenAi(String::from(
        "dall-e-2",
    ))));
    let identity = FakeIdentity::new(
        Some("llama-3"),
        ProviderIdentity::Custom(String::from("groq")),
    );
    let case = Case::new(&ops, &identity, Settings::default());
    case.run(call(Some("llama-3"), Some("groq"))).0.unwrap();
    let sent = ops.sends();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].branch, "openai");
    assert_eq!(sent[0].model.as_deref(), Some("llama-3"));
}

// a resolver that answers no model is impossible after normal resolution, so it is injected here
#[rstest]
#[case::black_forest_labs("black_forest_labs", ProviderId::BlackForestLabs, true)]
#[case::bedrock("bedrock", ProviderId::Bedrock, false)]
fn special_branches_need_a_model(
    #[case] slug: &str,
    #[case] provider: ProviderId,
    #[case] asynchronous: bool,
) {
    let ops = FakeOps::new(None);
    let unresolved = FakeIdentity::new(None, ProviderIdentity::Known(provider));
    match Case::new(&ops, &unresolved, Settings::default())
        .run(call(Some("model"), Some(slug)))
        .0
    {
        Err(ImageGenerationException::Mapped { class, message }) => {
            assert_eq!(message, format!("Model needs to be set for {slug}"));
            assert_plain_value_error(&class);
        }
        other => panic!("expected the missing model error, got {other:?}"),
    }
    assert!(ops.sends().is_empty());
    let identity = FakeIdentity::new(Some("resolved"), ProviderIdentity::Known(provider));
    let case = Case::new(&ops, &identity, Settings::default());
    let mut forwarded = call(Some("model"), Some(slug));
    forwarded.kwargs = kwargs(vec![("aimg_generation", Json::Bool(asynchronous))]);
    case.run(forwarded).0.unwrap();
    let sent = ops.sends();
    assert_eq!(sent[0].branch, slug);
    assert_eq!(sent[0].model.as_deref(), Some("resolved"));
    assert_eq!(sent[0].asynchronous, asynchronous);
}

// LiteLLM's openai branch takes api_key or the resolver's dynamic key
#[test]
fn resolved_key_reaches_the_openai_branch() {
    let openai = FakeOps::new(Some(ImageGenerationConfigTag::OpenAi(String::from(
        "dall-e-3",
    ))));
    let identity = FakeIdentity::keyed(
        Some("dall-e-3"),
        ProviderIdentity::Known(ProviderId::Openai),
    );
    Case::new(&openai, &identity, Settings::default())
        .run(call(Some("dall-e-3"), Some("openai")))
        .0
        .unwrap();
    assert_eq!(openai.sends()[0].api_key.as_deref(), Some("resolved-key"));
}

// no other branch reads the resolver's dynamic key
#[test]
fn resolved_key_skips_the_other_branches() {
    let azure = FakeOps::new(Some(ImageGenerationConfigTag::Azure(String::from("dep"))));
    let azure_identity =
        FakeIdentity::keyed(Some("dep"), ProviderIdentity::Known(ProviderId::Azure));
    Case::new(&azure, &azure_identity, Settings::default())
        .run(call(Some("dep"), Some("azure")))
        .0
        .unwrap();
    let azure_sent = azure.sends();
    assert!(azure_sent[0].api_key.is_none());
    assert!(azure_sent[0].header("api-key").is_none());

    let foundry = FakeOps::new(Some(ImageGenerationConfigTag::AzureAi(String::from(
        "flux",
    ))));
    let foundry_identity =
        FakeIdentity::keyed(Some("flux"), ProviderIdentity::Known(ProviderId::AzureAi));
    Case::new(&foundry, &foundry_identity, Settings::default())
        .run(call(Some("flux"), Some("azure_ai")))
        .0
        .unwrap();
    assert!(foundry.sends()[0].api_key.is_none());

    let handler = Arc::new(FixtureHandler::default());
    let custom = FakeOps::new(None);
    let custom_identity = FakeIdentity::keyed(
        Some("custom-model"),
        ProviderIdentity::Custom(String::from("myprov")),
    );
    let handler_case = custom_case(
        &custom,
        &custom_identity,
        vec![Some(Arc::<FixtureHandler>::clone(&handler))],
    );
    handler_case
        .run(call(Some("custom-model"), Some("myprov")))
        .0
        .unwrap();
    assert!(handler.seen()[0].api_key.is_none());
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

fn custom_case<'case>(
    ops: &'case FakeOps,
    identity: &'case FakeIdentity,
    handlers: Vec<Option<Arc<dyn CustomImageHandler>>>,
) -> Case<'case> {
    let mut case = Case::new(ops, identity, Settings::default());
    case.custom_providers.insert(String::from("myprov"));
    case.custom_provider_map = handlers
        .into_iter()
        .map(|handler| custom_provider("myprov", handler))
        .collect();
    case
}

fn custom_call(client: Option<CustomClientKind>, asynchronous: bool) -> ImageGenerationCall {
    let mut custom = call(Some("custom-model"), Some("myprov"));
    custom.client_kind = client;
    custom.kwargs = kwargs(vec![("aimg_generation", Json::Bool(asynchronous))]);
    custom
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
    let case = custom_case(
        &ops,
        &identity,
        vec![
            Some(Arc::<FixtureHandler>::clone(&first)),
            Some(Arc::<FixtureHandler>::clone(&last)),
        ],
    );
    case.run(custom_call(Some(CustomClientKind::Sync), false))
        .0
        .unwrap();
    assert_eq!(first.calls(), Vec::new());
    assert_eq!(last.calls(), vec![Some(CustomClientKind::Sync)]);
    assert_eq!(last.seen()[0].provider, "myprov");
    assert_eq!(last.seen()[0].model.as_deref(), Some("custom-model"));
    assert!(!last.seen()[0].asynchronous);
    custom_client_kinds(&identity);
    custom_handler_failures(&identity);
}

// each leg keeps only its own client class and hands the handler None for the other one
fn custom_client_kinds(identity: &FakeIdentity) {
    let ops = FakeOps::new(None);
    let wanted = Arc::new(FixtureHandler::default());
    let case = custom_case(
        &ops,
        identity,
        vec![Some(Arc::<FixtureHandler>::clone(&wanted))],
    );
    for (client, asynchronous) in [
        (CustomClientKind::Asynchronous, true),
        (CustomClientKind::Sync, true),
        (CustomClientKind::Asynchronous, false),
    ] {
        case.run(custom_call(Some(client), asynchronous)).0.unwrap();
    }
    assert_eq!(
        wanted.calls(),
        vec![Some(CustomClientKind::Asynchronous), None, None]
    );
    assert!(wanted.seen()[0].asynchronous);
}

fn custom_handler_failures(identity: &FakeIdentity) {
    let ops = FakeOps::new(None);
    let no_handler = custom_case(&ops, identity, vec![None]);
    match no_handler.run(call(Some("custom-model"), Some("myprov"))).0 {
        Err(ImageGenerationException::UnknownProvider { message }) => {
            assert!(message.contains("Unmapped LLM provider for this endpoint"));
            assert!(message.contains("model=custom-model"));
            assert!(message.contains("custom_llm_provider=myprov"));
        }
        other => panic!("expected the unknown provider error, got {other:?}"),
    }
    let failing = FakeOps::new(None);
    let failing_case = custom_case(
        &failing,
        identity,
        vec![Some(Arc::new(FixtureHandler::failing(429, "slow down")))],
    );
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
    api_key: Option<String>,
    asynchronous: bool,
}

struct FixtureHandler {
    calls: Mutex<Vec<Option<CustomClientKind>>>,
    seen: Mutex<Vec<HandlerSeen>>,
    failure: Option<(u16, String)>,
}

impl Default for FixtureHandler {
    fn default() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            seen: Mutex::new(Vec::new()),
            failure: None,
        }
    }
}

impl FixtureHandler {
    fn failing(status: u16, message: &str) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            seen: Mutex::new(Vec::new()),
            failure: Some((status, message.to_owned())),
        }
    }

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
                api_key: request
                    .api_key
                    .map(|secret| secret.expose_secret().to_owned()),
                asynchronous: request.asynchronous,
            });
        let failure = self.failure.clone();
        Box::pin(async move {
            match failure {
                Some((status, message)) => Err(Box::new(provider_failure(
                    status,
                    &message,
                    "custom-model",
                    "myprov",
                    "RateLimitError",
                ))),
                None => Ok(BranchAnswer {
                    response: ImageResponse {
                        created: NOW,
                        data: Some(Vec::new()),
                        usage: Some(ImageUsage::default()),
                    },
                    hidden_model: None,
                }),
            }
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
    let answered = answer.expect("the call answers");
    assert_eq!(answered.hidden_model.as_deref(), Some("dall-e-3"));
    assert_eq!(logging.user.as_deref(), Some("u"));
    assert_eq!(
        logging.model_call_details.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
    assert!(matches!(
        logging.litellm_params.get("timeout"),
        Some(Json::Number(number)) if number.as_f64() == Some(600.0_f64)
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
    let answered = answer.expect("the mock answers");
    let url = answered
        .response
        .data
        .as_ref()
        .and_then(|data| data[0].url.clone());
    assert_eq!(url, Some(String::new()));
    assert_eq!(
        logging.model_call_details.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
    // LiteLLM maps the options before it checks mock_response, so the provider config is selected first
    assert_eq!(
        ops.configs(),
        vec![(Some(String::from("dall-e-2")), String::from("openai"))]
    );
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
        Err(ImageGenerationException::Mapped { message, .. }) => assert_eq!(
            message,
            "GetLLMProvider Exception - LLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed model=unknown model\n\noriginal model: missing"
        ),
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
fn model_alone_still_resolves() {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::OpenAi(String::from(
        "dall-e-3",
    ))));
    let identity = FakeIdentity::new(
        Some("dall-e-3"),
        ProviderIdentity::Known(ProviderId::Openai),
    );
    let case = Case::new(&ops, &identity, Settings::default());
    case.run(call(Some("dall-e-3"), None)).0.unwrap();
    let seen = identity.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].model.as_deref(), Some("dall-e-3"));
    assert_eq!(seen[0].provider, None);
    assert_eq!(seen[0].api_base, None);
    assert!(!seen[0].has_key);
}

#[rstest]
#[case::drop_params("drop_params", Json::Bool(true))]
#[case::additional_drop_params(
    "additional_drop_params",
    Json::List(vec![Json::Text(String::from("imageConfig"))])
)]
fn drop_settings_let_unsupported_options_through(#[case] setting: &str, #[case] value: Json) {
    let ops = FakeOps::new(Some(ImageGenerationConfigTag::Recraft(String::from(
        "recraft-v3",
    ))));
    let identity = FakeIdentity::new(
        Some("recraft-v3"),
        ProviderIdentity::Known(ProviderId::Recraft),
    );
    let case = Case::new(&ops, &identity, Settings::default());
    let mut dropped = call(Some("recraft-v3"), Some("recraft"));
    dropped.kwargs = kwargs(vec![
        ("imageConfig", Json::Text(String::from("x"))),
        ("tenant_id", Json::Text(String::from("t"))),
        (setting, value),
    ]);
    case.run(dropped).0.unwrap();
    let sent = ops.sends();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].option("imageConfig").is_none());
    assert!(sent[0].option("tenant_id").is_none());
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
    let request = ImageResolutionRequest {
        model: None,
        custom_llm_provider: Some("openai"),
        api_base: None,
        api_key: None,
    };
    let error = identity
        .resolve(&request)
        .expect_err("a call without a model is rejected");
    assert_eq!(
        error.to_string(),
        "model parameter is required but was None. Please provide a valid model name."
    );
}
