use super::*;

#[tokio::test]
async fn unsupported_file_content() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("cohere"));
    let failure = run(&read, ContentEntry::Sync, &call, bare())
        .await
        .err()
        .unwrap();
    assert!(matches!(
        &failure,
        FileContentError::Unsupported { provider } if provider == "cohere"
    ));
    assert_eq!(
        failure.to_string(),
        "LiteLLM doesn't support cohere for 'file_content'. Supported providers are 'openai', 'azure', 'vertex_ai', 'bedrock', 'manus', 'anthropic'."
    );
    let placeholder = failure.bad_request().unwrap();
    assert_eq!(placeholder.model, "n/a");
    assert_eq!(placeholder.llm_provider, "cohere");
    assert_eq!(placeholder.body, "Unsupported provider");
    assert_eq!(placeholder.method, "CREATE_THREAD");
    assert_eq!(placeholder.url, "https://github.com/BerriAI/litellm");
    assert_eq!(failure.bad_request().map(|_request| 400_u16), Some(400));
    assert_eq!(sent(&read), Vec::new());
}

#[tokio::test]
async fn unsupported_file_content_unknown_provider_text() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("bogus"));
    let failure = run(&read, ContentEntry::Sync, &call, bare())
        .await
        .err()
        .unwrap();
    assert!(matches!(
        &failure,
        FileContentError::UnknownProvider { provider } if provider == "'bogus'"
    ));
    assert_eq!(failure.to_string(), "'bogus' is not a valid LlmProviders");
    assert_eq!(failure.bad_request().map(|_request| 400_u16), None);
    assert_eq!(sent(&read), Vec::new());
}

#[tokio::test]
async fn unsupported_file_content_stream_flag_does_not_change_the_branch() {
    let env = Env::default();
    let resolution = env.resolution();
    let read = reader(&resolution);
    let kwargs = ContentKwargs::default();
    let mut call = content_call(&kwargs);
    call.custom_llm_provider = Some(String::from("cohere"));
    call.stream = true;
    let failure = run(&read, ContentEntry::Sync, &call, bare())
        .await
        .err()
        .unwrap();
    assert!(matches!(failure, FileContentError::Unsupported { .. }));
}
