use super::*;

#[tokio::test]
async fn file_content_defaults() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.file_id = "file-1";
    let answer = run(&read, ContentEntry::Async, &call, bare())
        .await
        .unwrap();
    let content = answered_buffered(answer);
    assert_eq!(content.body, vec![0, 255]);
    assert_eq!(content.status, 200);
    let expected_request = FileContentRequest {
        file_id: String::from("file-1"),
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    };
    assert_eq!(
        sent(&read),
        vec![Sent {
            target: Seen::Compatible {
                api_base: String::from("https://api.openai.com/v1"),
                api_key: None,
                organization: None,
                max_retries: None,
                mode: ContentMode {
                    is_async: true,
                    timeout: FineTuningTimeout::Seconds(600.0_f64),
                },
                client: None,
            },
            request: expected_request,
            chunk_size: None,
        }]
    );
}

#[tokio::test]
async fn file_content_defaults_sync_provider_is_none() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let failure = run(&read, ContentEntry::Sync, &content_call(&kwargs), bare())
        .await
        .err()
        .unwrap();
    assert!(matches!(
        &failure,
        FileContentError::UnknownProvider { provider } if provider == "None"
    ));
    assert_eq!(failure.to_string(), "None is not a valid LlmProviders");
    assert_eq!(failure.bad_request().map(|_request| 400_u16), None);
    assert_eq!(sent(&read), Vec::new());
}

#[tokio::test]
async fn file_content_defaults_model_resolution_renames_the_provider() {
    let env = Env {
        catalog: ModelCatalog::default(),
        names: BTreeSet::from([String::from("openai")]),
        environment: BTreeMap::new(),
    };
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.model = Some("openai/gpt-4o");
    run(&read, ContentEntry::Sync, &call, bare()).await.unwrap();
    assert!(matches!(
        sent(&read).pop().unwrap().target,
        Seen::Compatible { .. }
    ));
}

#[tokio::test]
async fn file_content_defaults_resolution_errors_keep_the_declared_provider() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.model = Some("no-such-model");
    call.custom_llm_provider = Some(String::from("cohere"));
    let failure = run(&read, ContentEntry::Sync, &call, bare())
        .await
        .err()
        .unwrap();
    assert!(matches!(
        &failure,
        FileContentError::Unsupported { provider } if provider == "cohere"
    ));
    let mut unresolved = content_call(&kwargs);
    unresolved.model = Some("no-such-model");
    let swallowed = run(&read, ContentEntry::Sync, &unresolved, bare())
        .await
        .err()
        .unwrap();
    assert!(matches!(
        &swallowed,
        FileContentError::UnknownProvider { provider } if provider == "None"
    ));
}

#[tokio::test]
async fn file_content_defaults_trusted_credentials_precede_key_and_base() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let trusted = TrustedCredentials {
        values: BTreeMap::from([(String::from("bucket"), SecretString::from("files"))]),
    };
    let kwargs = ContentKwargs {
        api_key: Some(SecretString::from("sk-call")),
        api_base: Some(String::from("https://call.example")),
        api_version: None,
        organization: None,
        timeout: None,
        request_timeout: None,
        max_retries: None,
        litellm_call_id: None,
        id: None,
    };
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("anthropic"));
    run(
        &read,
        ContentEntry::Sync,
        &call,
        ContentContext {
            trusted: Some(&trusted),
            client: None,
            logger: None,
            logging: None,
        },
    )
    .await
    .unwrap();
    let generic = sent(&read).pop().unwrap().target.generic();
    assert_eq!(generic.trusted, Some(vec![String::from("bucket")]));
    assert_eq!(generic.api_key.as_deref(), Some("sk-call"));
    assert_eq!(generic.api_base.as_deref(), Some("https://call.example"));
}

#[tokio::test]
async fn file_content_defaults_trusted_credentials_replace_null_key_and_base() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let trusted = TrustedCredentials {
        values: BTreeMap::from([(String::from("bucket"), SecretString::from("files"))]),
    };
    let null_kwargs = ContentKwargs::default();
    let mut null_call = content_call(&null_kwargs);
    null_call.custom_llm_provider = Some(String::from("anthropic"));
    run(
        &read,
        ContentEntry::Sync,
        &null_call,
        ContentContext {
            trusted: Some(&trusted),
            client: None,
            logger: None,
            logging: None,
        },
    )
    .await
    .unwrap();
    let overwritten = sent(&read).pop().unwrap().target.generic();
    assert_eq!(overwritten.api_key, None);
    assert_eq!(overwritten.api_base, None);
    assert_eq!(overwritten.trusted, Some(vec![String::from("bucket")]));
}

fn number(value: u32) -> ReadTimeoutValue {
    ReadTimeoutValue::Number(Number::from(value))
}

fn text(value: &str) -> ReadTimeoutValue {
    ReadTimeoutValue::Text(value.to_owned())
}

fn phases(read: Option<u32>) -> ReadTimeout {
    ReadTimeout::Structured(ReadStructuredTimeout {
        connect: None,
        read: read.map(Number::from),
        write: None,
        pool: None,
    })
}

const fn kept(read: Option<f64>) -> FineTuningTimeout {
    FineTuningTimeout::Structured(StructuredTimeout {
        connect: None,
        read,
        write: None,
        pool: None,
    })
}

#[rstest]
#[case::timeout_over_request_timeout(
    Some(ReadTimeout::Plain(number(2))),
    Some(number(3)),
    "openai",
    FineTuningTimeout::Seconds(2.0_f64)
)]
#[case::request_timeout_alone(None, Some(number(3)), "openai", FineTuningTimeout::Seconds(3.0_f64))]
#[case::both_absent(None, None, "openai", FineTuningTimeout::Seconds(600.0_f64))]
#[case::zero_falls_through(
    Some(ReadTimeout::Plain(number(0))),
    Some(number(3)),
    "openai",
    FineTuningTimeout::Seconds(3.0_f64)
)]
#[case::empty_text_falls_through(
    Some(ReadTimeout::Plain(text(""))),
    Some(number(3)),
    "openai",
    FineTuningTimeout::Seconds(3.0_f64)
)]
#[case::numeric_text(
    Some(ReadTimeout::Plain(text("2.5"))),
    None,
    "openai",
    FineTuningTimeout::Seconds(2.5_f64)
)]
#[case::openai_keeps_phases(Some(phases(Some(5))), Some(number(3)), "openai", kept(Some(5.0_f64)))]
#[case::azure_keeps_phases(Some(phases(Some(5))), None, "azure", kept(Some(5.0_f64)))]
#[case::bedrock_keeps_phases(Some(phases(Some(5))), None, "bedrock", kept(Some(5.0_f64)))]
#[case::anthropic_reads_the_read_phase(
    Some(phases(Some(5))),
    None,
    "anthropic",
    FineTuningTimeout::Seconds(5.0_f64)
)]
#[case::anthropic_without_read_phase(
    Some(phases(None)),
    None,
    "anthropic",
    FineTuningTimeout::Seconds(600.0_f64)
)]
#[tokio::test]
async fn file_content_defaults_timeout_selection(
    #[case] timeout: Option<ReadTimeout>,
    #[case] request_timeout: Option<ReadTimeoutValue>,
    #[case] provider: &str,
    #[case] expected: FineTuningTimeout,
) {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs {
        api_key: None,
        api_base: None,
        api_version: None,
        organization: None,
        timeout,
        request_timeout,
        max_retries: None,
        litellm_call_id: None,
        id: None,
    };
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(provider.to_owned());
    run(&read, ContentEntry::Sync, &call, bare()).await.unwrap();
    assert_eq!(sent(&read).pop().unwrap().target.mode().timeout, expected);
}

#[rstest]
#[case::unsupported_provider("cohere")]
#[case::unknown_provider("bogus")]
#[tokio::test]
async fn file_content_defaults_bad_text_fails_before_the_provider(#[case] provider: &str) {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs {
        api_key: None,
        api_base: None,
        api_version: None,
        organization: None,
        timeout: Some(ReadTimeout::Plain(text("soon"))),
        request_timeout: None,
        max_retries: None,
        litellm_call_id: None,
        id: None,
    };
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(provider.to_owned());
    let failure = run(&read, ContentEntry::Sync, &call, bare())
        .await
        .err()
        .unwrap();
    assert!(matches!(&failure, FileContentError::Timeout(_)));
    assert_eq!(failure.bad_request().map(|_request| 400_u16), None);
    assert_eq!(sent(&read), Vec::new());
}
