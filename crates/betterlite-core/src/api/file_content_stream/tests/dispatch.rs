use super::*;

#[rstest]
#[case::openai("openai")]
#[case::hosted_vllm("hosted_vllm")]
#[case::litellm_proxy("litellm_proxy")]
#[tokio::test]
async fn buffered_content_dispatch_compatible(#[case] provider: &str) {
    let env = Env::default();
    let resolution = env.resolution();
    let mut settings = default_settings();
    settings.openai_api_key = Some(SecretString::from("sk-settings"));
    settings.openai_organization = Some(String::from("org-settings"));
    let read = service(&resolution, settings, &[]);
    let kwargs = ContentKwargs {
        api_key: None,
        api_base: None,
        api_version: None,
        organization: None,
        timeout: None,
        request_timeout: None,
        max_retries: Some(4),
        litellm_call_id: None,
        id: None,
    };
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(provider.to_owned());
    let answer = run(&read, ContentEntry::Sync, &call, bare()).await.unwrap();
    assert_eq!(answered_buffered(answer).body, vec![0, 255]);
    let record = sent(&read).pop().unwrap();
    assert_eq!(
        record.target,
        Seen::Compatible {
            api_base: String::from("https://api.openai.com/v1"),
            api_key: Some(String::from("sk-settings")),
            organization: Some(String::from("org-settings")),
            max_retries: Some(4),
            mode: ContentMode {
                is_async: false,
                timeout: FineTuningTimeout::Seconds(600.0_f64),
            },
            client: None,
        }
    );
    assert_eq!(record.request.file_id, "file-1");
}

#[tokio::test]
async fn buffered_content_dispatch_forwards_headers_and_body_without_a_timeout() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("openai"));
    call.extra_headers = Some(text_map(&[("x-trace", "1")]));
    call.extra_body = Some(text_map(&[("dropped", "1")]));
    run(&read, ContentEntry::Sync, &call, bare()).await.unwrap();
    assert_eq!(
        sent(&read).pop().unwrap().request,
        FileContentRequest {
            file_id: String::from("file-1"),
            extra_headers: IndexMap::from([(String::from("x-trace"), String::from("1"))]),
            extra_body: IndexMap::from([(String::from("dropped"), String::from("1"))]),
            timeout: None,
        }
    );
}

#[tokio::test]
async fn buffered_content_dispatch_azure() {
    let env = Env::default();
    let resolution = env.resolution();
    let mut settings = default_settings();
    settings.azure_api_key = Some(SecretString::from("sk-azure"));
    settings.azure_api_version = Some(String::from("2024-06-01"));
    let read = service(&resolution, settings, &[]);
    let kwargs = ContentKwargs {
        api_key: None,
        api_base: Some(String::from("https://azure.example")),
        api_version: None,
        organization: None,
        timeout: None,
        request_timeout: None,
        max_retries: None,
        litellm_call_id: None,
        id: None,
    };
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("azure"));
    let context = sdk_context(SuppliedSdk::Azure);
    run(&read, ContentEntry::Async, &call, context)
        .await
        .unwrap();
    assert_eq!(
        sent(&read).pop().unwrap().target,
        Seen::Azure {
            api_base: Some(String::from("https://azure.example")),
            api_key: Some(String::from("sk-azure")),
            api_version: Some(String::from("2024-06-01")),
            max_retries: None,
            mode: ContentMode {
                is_async: true,
                timeout: FineTuningTimeout::Seconds(600.0_f64),
            },
            client: Some(SuppliedSdk::Azure),
        }
    );
}

#[rstest]
#[case::anthropic(ProviderId::Anthropic)]
#[case::bedrock(ProviderId::Bedrock)]
#[case::vertex_ai(ProviderId::VertexAi)]
#[tokio::test]
async fn buffered_content_dispatch_generic_configs(#[case] provider: ProviderId) {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let trusted = TrustedCredentials {
        values: BTreeMap::from([(String::from("bucket"), SecretString::from("files"))]),
    };
    let kwargs = ContentKwargs {
        api_key: Some(SecretString::from("sk-call")),
        api_base: None,
        api_version: None,
        organization: None,
        timeout: None,
        request_timeout: None,
        max_retries: None,
        litellm_call_id: None,
        id: None,
    };
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(<&'static str>::from(provider).to_owned());
    let context = http_context(ClientKind::Asynchronous, Some(&trusted));
    run(&read, ContentEntry::Sync, &call, context)
        .await
        .unwrap();
    let generic = sent(&read).pop().unwrap().target.generic();
    assert_eq!(generic.config, FakeConfig(provider));
    assert_eq!(generic.client, Some(ClientKind::Asynchronous));
    assert_eq!(generic.trusted, Some(vec![String::from("bucket")]));
    assert_eq!(generic.api_key.as_deref(), Some("sk-call"));
    assert_eq!(
        generic.created,
        Some(NewLogger {
            call_type: "file_content",
            start_time: DateTime::<FixedOffset>::parse_from_rfc3339(STARTED).unwrap(),
            litellm_call_id: String::from(GENERATED_ID),
            function_id: String::new(),
        })
    );
}

#[tokio::test]
async fn buffered_content_dispatch_generic_drops_sdk_clients() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("anthropic"));
    let context = sdk_context(SuppliedSdk::OpenAi);
    run(&read, ContentEntry::Sync, &call, context)
        .await
        .unwrap();
    assert_eq!(sent(&read).pop().unwrap().target.generic().client, None);
}

#[tokio::test]
async fn buffered_content_dispatch_compatible_drops_every_client() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("openai"));
    let context = sdk_context(SuppliedSdk::OpenAi);
    run(&read, ContentEntry::Sync, &call, context)
        .await
        .unwrap();
    assert_eq!(
        sent(&read).pop().unwrap().target,
        Seen::Compatible {
            api_base: String::from("https://api.openai.com/v1"),
            api_key: None,
            organization: None,
            max_retries: None,
            mode: ContentMode {
                is_async: false,
                timeout: FineTuningTimeout::Seconds(600.0_f64),
            },
            client: None,
        }
    );
}

#[tokio::test]
async fn buffered_content_dispatch_xai_batch_precedes_config() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = service(
        &resolution,
        Settings::default(),
        &[ProviderId::Xai, ProviderId::Anthropic],
    );
    let kwargs = ContentKwargs {
        api_key: Some(SecretString::from("sk-xai")),
        api_base: Some(String::from("https://xai.example")),
        api_version: None,
        organization: None,
        timeout: None,
        request_timeout: None,
        max_retries: None,
        litellm_call_id: None,
        id: None,
    };
    let mut call = content_call(&kwargs);
    call.file_id = "batch_7";
    call.custom_llm_provider = Some(String::from("xai"));
    run(&read, ContentEntry::Async, &call, bare())
        .await
        .unwrap();
    assert_eq!(
        sent(&read).pop().unwrap().target,
        Seen::XaiBatch {
            batch_id: String::from("batch_7"),
            api_base: Some(String::from("https://xai.example")),
            api_key: Some(String::from("sk-xai")),
            timeout: FineTuningTimeout::Seconds(600.0_f64),
        }
    );
}

#[tokio::test]
async fn buffered_content_dispatch_ordinary_xai_file_uses_its_config() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = service(&resolution, Settings::default(), &[ProviderId::Xai]);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.file_id = "file-7";
    call.custom_llm_provider = Some(String::from("xai"));
    run(&read, ContentEntry::Sync, &call, bare()).await.unwrap();
    assert_eq!(
        sent(&read).pop().unwrap().target.generic().config,
        FakeConfig(ProviderId::Xai)
    );
}

#[rstest]
#[case::not_found(404)]
#[case::rate_limited(429)]
#[tokio::test]
async fn buffered_content_dispatch_provider_errors_survive(#[case] status: u16) {
    let env = Env::default();
    let resolution = env.resolution();
    let mut read = reader(&resolution);
    read.dispatch.failure = Some(FakeFailure(status));
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("openai"));
    let failure = run(&read, ContentEntry::Sync, &call, bare())
        .await
        .err()
        .unwrap();
    assert!(
        matches!(&failure, FileContentError::Dispatch(FakeFailure(answered)) if *answered == status)
    );
    assert_eq!(failure.bad_request().map(|_request| 400_u16), None);
    assert_eq!(sent(&read).len(), 1);
}
#[tokio::test]
async fn content_reply_kinds_are_checked() {
    let env = Env::default();
    let resolution = env.resolution();
    let mut read = reader(&resolution);
    read.dispatch.wrong_reply = true;
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("openai"));
    let buffered = run(&read, ContentEntry::Sync, &call, bare())
        .await
        .err()
        .unwrap();
    assert!(matches!(buffered, FileContentError::Reply));
    call.stream = true;
    let streamed = run(&read, ContentEntry::Sync, &call, bare())
        .await
        .err()
        .unwrap();
    assert!(matches!(streamed, FileContentError::Reply));
}
#[tokio::test]
async fn buffered_content_dispatch_keeps_a_supplied_logging_state() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"files","call_type":"file_content","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1"}"#,
    )
    .unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        Duration::default,
    )
    .unwrap();
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("anthropic"));
    let context = ContentContext {
        trusted: None,
        client: None,
        logger: Some(&mut state),
        logging: None,
    };
    run(&read, ContentEntry::Sync, &call, context)
        .await
        .unwrap();
    assert_eq!(sent(&read).pop().unwrap().target.generic().created, None);
}

// LiteLLM forwards any non-None client to the Azure SDK handler, where an httpx client fails
#[tokio::test]
async fn buffered_content_dispatch_azure_drops_an_http_handler() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("azure"));
    let context = http_context(ClientKind::Asynchronous, None);
    run(&read, ContentEntry::Async, &call, context)
        .await
        .unwrap();
    assert!(matches!(
        sent(&read).pop().unwrap().target,
        Seen::Azure { client: None, .. }
    ));
}
