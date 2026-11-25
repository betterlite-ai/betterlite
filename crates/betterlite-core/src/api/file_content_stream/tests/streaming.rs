use super::super::streaming::FileContentStreaming as _;
use super::*;

fn written_identity(model: &str, provider: &str, api_base: Option<&str>) -> RecordedIdentity {
    RecordedIdentity {
        model: model.to_owned(),
        custom_llm_provider: provider.to_owned(),
        api_base: api_base.map(str::to_owned),
    }
}

const fn content_kwargs(
    api_base: Option<String>,
    litellm_call_id: Option<String>,
    id: Option<String>,
) -> ContentKwargs {
    ContentKwargs {
        api_key: None,
        api_base,
        api_version: None,
        organization: None,
        timeout: None,
        request_timeout: None,
        max_retries: None,
        litellm_call_id,
        id,
    }
}

#[tokio::test]
async fn streaming_logging_identity_keeps_a_supplied_object() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let logger = Arc::new(RecordingLogging::default());
    let kwargs = content_kwargs(Some(String::from("https://call.example")), None, None);
    let mut call = content_call(&kwargs);
    call.model = Some("gpt-4o");
    call.custom_llm_provider = Some(String::from("openai"));
    call.stream = true;
    let context = logging_context(&logger);
    let mut body = answered_streamed(
        run(&read, ContentEntry::Async, &call, context)
            .await
            .unwrap(),
    );
    assert!(read.dispatch.created.lock().unwrap().is_empty());
    assert_eq!(
        logger.identities(),
        vec![written_identity(
            "gpt-4o",
            "openai",
            Some("https://call.example")
        )]
    );
    assert_eq!(
        body.headers.get("x-file").map(String::as_str),
        Some("capture")
    );
    assert_eq!(drained(&mut body).await, vec![vec![0, 255]]);
    assert_eq!(logger.successes(), 1);
    assert_eq!(logger.failures(), 0);
    assert_eq!(logger.success_times(), vec![(started(), started())]);
}

#[rstest]
#[case::absent(None, None)]
#[case::empty(Some(""), Some(""))]
#[case::model(Some("gpt-4o"), Some("gpt-4o"))]
#[tokio::test]
async fn streaming_logging_identity_normalizes_the_model(
    #[case] model: Option<&str>,
    #[case] written: Option<&str>,
) {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let logger = Arc::new(RecordingLogging::default());
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.model = model;
    call.custom_llm_provider = Some(String::from("openai"));
    call.stream = true;
    let context = logging_context(&logger);
    run(&read, ContentEntry::Sync, &call, context)
        .await
        .unwrap();
    assert_eq!(
        logger.identities(),
        vec![written_identity(
            written.unwrap_or_default(),
            "openai",
            None
        )]
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[case::value(Some("https://call.example"))]
#[tokio::test]
async fn streaming_logging_identity_writes_only_a_non_null_api_base(
    #[case] api_base: Option<&str>,
) {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let logger = Arc::new(RecordingLogging::default());
    let kwargs = content_kwargs(api_base.map(str::to_owned), None, None);
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("hosted_vllm"));
    call.stream = true;
    let context = logging_context(&logger);
    run(&read, ContentEntry::Sync, &call, context)
        .await
        .unwrap();
    assert_eq!(
        logger.identities(),
        vec![written_identity("", "hosted_vllm", api_base)]
    );
}

#[rstest]
#[case::generated(None, None, "afile_content", GENERATED_ID, "")]
#[case::given_call_id(Some("call-7"), None, "afile_content", "call-7", "")]
#[case::empty_call_id(Some(""), None, "afile_content", GENERATED_ID, "")]
#[case::given_function_id(None, Some("fn-1"), "afile_content", GENERATED_ID, "fn-1")]
#[case::sync_call_type(None, None, "file_content", GENERATED_ID, "")]
#[tokio::test]
async fn streaming_logging_identity_creates_the_missing_object(
    #[case] litellm_call_id: Option<&str>,
    #[case] id: Option<&str>,
    #[case] call_type: &'static str,
    #[case] call_id: &str,
    #[case] function_id: &str,
) {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = content_kwargs(
        None,
        litellm_call_id.map(str::to_owned),
        id.map(str::to_owned),
    );
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("openai"));
    call.stream = true;
    let entry = if call_type == "afile_content" {
        ContentEntry::Async
    } else {
        ContentEntry::Sync
    };
    run(&read, entry, &call, bare()).await.unwrap();
    assert_eq!(
        *read.dispatch.created.lock().unwrap(),
        vec![NewLogger {
            call_type,
            start_time: DateTime::<FixedOffset>::parse_from_rfc3339(STARTED).unwrap(),
            litellm_call_id: call_id.to_owned(),
            function_id: function_id.to_owned(),
        }]
    );
    let logger = Arc::clone(&read.dispatch.loggers.lock().unwrap()[0]);
    assert_eq!(
        logger.identities(),
        vec![written_identity("", "openai", None)]
    );
}

#[tokio::test]
async fn streaming_logging_identity_precedes_the_vertex_error() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let logger = Arc::new(RecordingLogging::default());
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.model = Some("gemini-2.0-flash");
    call.custom_llm_provider = Some(String::from("vertex_ai"));
    call.stream = true;
    let context = logging_context(&logger);
    let failure = run(&read, ContentEntry::Sync, &call, context)
        .await
        .err()
        .unwrap();
    assert!(matches!(failure, FileContentError::SyncVertexStreaming));
    assert_eq!(
        logger.identities(),
        vec![written_identity("gemini-2.0-flash", "vertex_ai", None)]
    );
    assert_eq!(sent(&read), Vec::new());
}

#[rstest]
#[case::openai("openai")]
#[case::hosted_vllm("hosted_vllm")]
#[case::litellm_proxy("litellm_proxy")]
#[tokio::test]
async fn sdk_stream_selection_compatible(#[case] provider: &str) {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(provider.to_owned());
    call.stream = true;
    call.chunk_size = 3;
    let context = sdk_context(SuppliedSdk::OpenAi);
    let mut body = answered_streamed(
        run(&read, ContentEntry::Async, &call, context)
            .await
            .unwrap(),
    );
    assert_eq!(drained(&mut body).await, vec![vec![0, 255]]);
    let record = sent(&read).pop().unwrap();
    assert_eq!(record.chunk_size, NonZeroUsize::new(3));
    assert!(matches!(
        record.target,
        Seen::Compatible {
            client: Some(SuppliedSdk::OpenAi),
            ..
        }
    ));
}

#[tokio::test]
async fn sdk_stream_selection_default_chunk_size_and_no_sdk_azure_client() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("openai"));
    call.stream = true;
    let context = sdk_context(SuppliedSdk::Azure);
    run(&read, ContentEntry::Sync, &call, context)
        .await
        .unwrap();
    let record = sent(&read).pop().unwrap();
    assert_eq!(record.chunk_size, NonZeroUsize::new(1024 * 1024));
    assert!(matches!(
        record.target,
        Seen::Compatible { client: None, .. }
    ));
}

#[tokio::test]
async fn sdk_stream_selection_zero_chunk_size_takes_the_default() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("openai"));
    call.stream = true;
    call.chunk_size = 0;
    run(&read, ContentEntry::Sync, &call, bare()).await.unwrap();
    assert_eq!(sent(&read).pop().unwrap().chunk_size, None);
}

#[tokio::test]
async fn sdk_stream_selection_vertex_needs_an_async_http_handler() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("vertex_ai"));
    call.stream = true;
    let context = http_context(ClientKind::Asynchronous, None);
    answered_streamed(
        run(&read, ContentEntry::Async, &call, context)
            .await
            .unwrap(),
    );
    let record = sent(&read).pop().unwrap();
    assert_eq!(
        record.target.generic().client,
        Some(ClientKind::Asynchronous)
    );
    assert_eq!(
        record.target.generic().config,
        FakeConfig(ProviderId::VertexAi)
    );
}

#[tokio::test]
async fn sdk_stream_selection_vertex_drops_a_sync_http_handler() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("vertex_ai"));
    call.stream = true;
    let context = http_context(ClientKind::Sync, None);
    answered_streamed(
        run(&read, ContentEntry::Async, &call, context)
            .await
            .unwrap(),
    );
    assert_eq!(sent(&read).pop().unwrap().target.generic().client, None);
}

#[rstest]
#[case::azure("azure")]
#[case::bedrock("bedrock")]
#[case::anthropic("anthropic")]
#[tokio::test]
async fn sdk_stream_selection_unsupported_names_stay_buffered(#[case] provider: &str) {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(provider.to_owned());
    call.stream = true;
    call.chunk_size = 3;
    let answer = run(&read, ContentEntry::Sync, &call, bare()).await.unwrap();
    answered_buffered(answer);
    let record = sent(&read).pop().unwrap();
    assert_eq!(record.chunk_size, None);
}

#[tokio::test]
async fn sdk_stream_selection_sync_vertex_rejects_the_placeholder() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("vertex_ai"));
    call.stream = true;
    let failure = run(&read, ContentEntry::Sync, &call, bare())
        .await
        .err()
        .unwrap();
    assert_eq!(
        failure.to_string(),
        "Streaming 'file_content' for vertex_ai is only supported through 'afile_content'."
    );
    let placeholder = failure.bad_request().unwrap();
    assert_eq!(placeholder.model, "n/a");
    assert_eq!(placeholder.body, "Unsupported provider");
    assert_eq!(placeholder.method, "FILE_CONTENT");
    assert_eq!(placeholder.llm_provider, "vertex_ai");
    assert_eq!(placeholder.url, "https://github.com/BerriAI/litellm");
    assert_eq!(failure.bad_request().map(|_request| 400_u16), Some(400));
    assert_eq!(sent(&read), Vec::new());
}

#[tokio::test]
async fn sdk_stream_selection_vertex_without_a_config_is_unsupported() {
    let env = Env::default();
    let resolution = env.resolution();
    let unconfigured = service(&resolution, Settings::default(), &[]);
    let kwargs = ContentKwargs::default();
    let call = content_call(&kwargs);
    let selection = Selection {
        provider: Some(String::from("vertex_ai")),
        mode: ContentMode {
            is_async: true,
            timeout: FineTuningTimeout::Seconds(600.0_f64),
        },
        request: FileContentRequest {
            file_id: String::from("file-1"),
            extra_headers: IndexMap::new(),
            extra_body: IndexMap::new(),
            timeout: None,
        },
    };
    let failure = unconfigured
        .streamed(&call, &selection, bare())
        .await
        .err()
        .unwrap();
    assert!(matches!(
        &failure,
        FileContentError::UnsupportedStreaming { provider } if provider == "vertex_ai"
    ));
    assert_eq!(sent(&unconfigured), Vec::new());
}
#[test]
fn streamed_source_failures_are_unclassified_exceptions() {
    let failure = stream_source_failure(&FilesError::NotAsync);
    let SourceFailure::Exception(ErrorClass::Unclassified(context)) = &failure else {
        panic!("not an unclassified exception: {failure:?}");
    };
    assert!(context.message.contains("AsyncOpenAI"));
}
