#[test]
fn the_pipeline_tag_source_reads_pipeline_tag_or_fails_500() {
    let found = PrefetchedTag(hub_reply(200, r#"{"pipeline_tag":"rerank"}"#));
    assert_eq!(found.tag("m", None).unwrap(), Some(String::from("rerank")));
    let absent = PrefetchedTag(hub_reply(200, r#"{"id":"m"}"#));
    assert_eq!(absent.tag("m", None).unwrap(), None);
    let error = PrefetchedTag(hub_reply(200, "not json"))
        .tag("m", None)
        .unwrap_err();
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "the Hugging Face model info is not JSON");
}

#[test]
fn polling_reads_the_replicate_variables_and_reports_a_bad_one() {
    let env = environment(&[
        ("DEFAULT_REPLICATE_POLLING_RETRIES", "2"),
        ("DEFAULT_REPLICATE_POLLING_DELAY_SECONDS", "4"),
        ("REPLICATE_POLLING_DELAY_SECONDS", "0.25"),
        ("REPLICATE_MODEL_NAME_WITH_ID_LENGTH", "8"),
    ]);
    let found = polling(&env).unwrap();
    assert_eq!(found.retries, 2_i32);
    assert_eq!(found.retry_delay_seconds.to_bits(), 4.0_f64.to_bits());
    assert_eq!(found.polling_delay_seconds.to_bits(), 0.25_f64.to_bits());
    assert_eq!(found.model_name_with_id_length, 8);
    let defaults: ReplicatePolling = polling(&environment(&[])).unwrap();
    assert_eq!(defaults.retries, 5_i32);
    let error = polling(&environment(&[(
        "DEFAULT_REPLICATE_POLLING_RETRIES",
        "five",
    )]))
    .expect_err("a text retry count is an environment error");
    let HubDispatchError::Environment(source) = error else {
        panic!("a bad polling value is an environment error");
    };
    assert!(
        source
            .to_string()
            .contains("DEFAULT_REPLICATE_POLLING_RETRIES")
    );
}

#[test]
fn the_cl100k_counter_counts_text() {
    let counter = Cl100k::new().unwrap();
    assert_eq!(counter.count("m", "hello world").unwrap(), 2);
    assert_eq!(counter.encode_ordinary("hello world"), 2);
}

#[tokio::test]
async fn the_embedding_arm_posts_the_prepared_body_and_logs_both_hooks() {
    let services = Services::new(
        ScriptedHub::new(Vec::new()),
        ScriptedPost::new(vec![http_reply(200, r#"[{"embedding":[0.5,0.25]}]"#)]),
        ScriptedTransport::new(Vec::new()),
    );
    let globals = empty_globals();
    let env = environment(&[("HUGGINGFACE_API_KEY", "hf-key")]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let input = vec![String::from("a"), String::from("b")];
    let options = object(&[
        (
            "input_type",
            PromptJson::Text(String::from("feature-extraction")),
        ),
        ("wait_for_model", PromptJson::Bool(true)),
    ]);
    let headers = Headers::from([(String::from("x-call"), String::from("1"))]);
    let call = EmbeddingCall {
        model: "org/model",
        input: &input,
        optional_params: &options,
        api_key: None,
        api_base: Some("http://mock/embed"),
        headers: &headers,
    };
    let response: EmbeddingResponse = embed(&call, &deps, &mut log).await.unwrap();
    assert_eq!(services.hub.calls(), 0);
    let requests = services.post.requests();
    assert_eq!(requests.len(), 1);
    let (url, sent_headers, sent_body) = requests.first().unwrap();
    assert_eq!(url, "http://mock/embed");
    assert_eq!(sent_headers.get("Authorization").unwrap(), "Bearer hf-key");
    assert_eq!(
        sent_headers.get("content-type").unwrap(),
        "application/json"
    );
    assert_eq!(sent_headers.get("x-call").unwrap(), "1");
    assert_eq!(
        *sent_body,
        String::from(r#"{"inputs":["a","b"],"options":{"wait_for_model":true}}"#)
    );
    assert_eq!(response.data.len(), 1);
    assert_eq!(response.usage.prompt_tokens, 2);
    assert_eq!(
        state.model_call_details.get("log_event_type"),
        Some(&Json::Text(String::from("post_api_call")))
    );
    let Some(Json::List(items)) = state.model_call_details.get("original_response") else {
        panic!("the post_call keeps the reply");
    };
    assert_eq!(items.len(), 1);
}

#[tokio::test]
async fn the_embedding_arm_reads_the_pipeline_tag_only_without_input_type() {
    let services = Services::new(
        ScriptedHub::new(vec![hub_reply(
            200,
            r#"{"pipeline_tag":"feature-extraction"}"#,
        )]),
        ScriptedPost::new(vec![http_reply(200, r#"[{"embedding":[0.5]}]"#)]),
        ScriptedTransport::new(Vec::new()),
    );
    let globals = empty_globals();
    let env = environment(&[]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let input = vec![String::from("a")];
    let options = Object::new();
    let headers = Headers::new();
    let call = EmbeddingCall {
        model: "org/model",
        input: &input,
        optional_params: &options,
        api_key: None,
        api_base: None,
        headers: &headers,
    };
    embed(&call, &deps, &mut log).await.unwrap();
    let requests = services.hub.requests();
    assert_eq!(requests.len(), 1);
    let (url, sent_headers, query) = requests.first().unwrap();
    assert_eq!(url, "https://huggingface.co/api/models/org/model");
    assert!(sent_headers.is_empty());
    assert!(query.is_empty());
    let posted = services.post.requests();
    assert_eq!(
        posted.first().unwrap().0,
        "https://router.huggingface.co/hf-inference/pipeline/feature-extraction/org/model"
    );
}

#[tokio::test]
async fn the_embedding_arm_fails_with_500_when_the_reply_has_an_error_key() {
    let services = Services::new(
        ScriptedHub::new(Vec::new()),
        ScriptedPost::new(vec![http_reply(200, r#"{"error":"boom"}"#)]),
        ScriptedTransport::new(Vec::new()),
    );
    let globals = empty_globals();
    let env = environment(&[]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let input = vec![String::from("a")];
    let options = object(&[(
        "input_type",
        PromptJson::Text(String::from("feature-extraction")),
    )]);
    let headers = Headers::new();
    let call = EmbeddingCall {
        model: "org/model",
        input: &input,
        optional_params: &options,
        api_key: None,
        api_base: Some("http://mock/embed"),
        headers: &headers,
    };
    let error = embed(&call, &deps, &mut log)
        .await
        .expect_err("the error key fails the call");
    let HubDispatchError::Embedding(HuggingfaceEmbeddingError::Huggingface(error)) = error else {
        panic!("the error key is a Hugging Face error");
    };
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "boom");
}

#[tokio::test]
async fn the_huggingface_arm_prepares_the_chat_request() {
    let services = Services::idle();
    let globals = empty_globals();
    let env = environment(&[("HF_TOKEN", "hf-token")]);
    let deps = services.deps(&globals, &env);
    let mut messages = vec![user_message()];
    let mut options = object(&[
        ("temperature", number(0)),
        ("stream", PromptJson::Bool(true)),
    ]);
    let headers = Headers::from([(String::from("x-call"), String::from("1"))]);
    let mut call = ChatContext {
        headers: Some(&headers),
        api_base: Some("http://mock/v1"),
        ..chat_context(
            "meta-llama/Llama-3-8B-Instruct",
            &mut messages,
            &mut options,
            Some("http://mock/v1"),
            None,
        )
    };
    let request = huggingface::chat_request(&mut call, &deps).await.unwrap();
    let PreparedRequest::Stream(prepared) = request else {
        panic!("a stream call prepares a stream request");
    };
    assert_eq!(
        prepared.http.url.as_str(),
        "http://mock/v1/chat/completions"
    );
    assert_eq!(
        prepared.http.headers.get("Authorization").unwrap(),
        "Bearer hf-token"
    );
    assert_eq!(
        prepared.http.headers.get("content-type").unwrap(),
        "application/json"
    );
    assert_eq!(prepared.http.headers.get("x-call").unwrap(), "1");
    let body: Object = serde_json::from_slice(&prepared.http.body).unwrap();
    assert_eq!(
        body.get("model"),
        Some(&PromptJson::Text(String::from(
            "meta-llama/Llama-3-8B-Instruct"
        )))
    );
    assert_eq!(body.get("stream"), Some(&PromptJson::Bool(true)));
    assert_eq!(body.get("temperature"), Some(&number(0)));
    assert_eq!(services.hub.calls(), 0);
    let mut plain_messages = vec![user_message()];
    let mut plain_options = Object::new();
    let mut plain = chat_context(
        "meta-llama/Llama-3-8B-Instruct",
        &mut plain_messages,
        &mut plain_options,
        Some("http://mock/v1"),
        None,
    );
    let plain_request = huggingface::chat_request(&mut plain, &deps).await.unwrap();
    assert!(matches!(plain_request, PreparedRequest::Chat(_)));
}

#[tokio::test]
async fn the_huggingface_arm_fetches_the_provider_mapping_without_a_base() {
    let mapping = r#"{"inferenceProviderMapping":{"hf-inference":{"status":"live","providerId":"meta-llama/Llama-3-8B-Instruct"}}}"#;
    let services = Services::new(
        ScriptedHub::new(vec![hub_reply(200, mapping)]),
        ScriptedPost::new(Vec::new()),
        ScriptedTransport::new(Vec::new()),
    );
    let globals = empty_globals();
    let env = environment(&[("HUGGINGFACE_API_KEY", "hub-secret")]);
    let deps = services.deps(&globals, &env);
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context(
        "hf-inference/meta-llama/Llama-3-8B-Instruct",
        &mut messages,
        &mut options,
        None,
        None,
    );
    let request = huggingface::chat_request(&mut call, &deps).await.unwrap();
    let PreparedRequest::Chat(prepared) = request else {
        panic!("a plain call prepares a chat request");
    };
    let requests = services.hub.requests();
    assert_eq!(requests.len(), 1);
    let (url, sent_headers, query) = requests.first().unwrap();
    assert_eq!(
        url,
        "https://huggingface.co/api/models/meta-llama/Llama-3-8B-Instruct"
    );
    assert_eq!(
        sent_headers.get("Authorization").unwrap(),
        "Bearer hub-secret"
    );
    assert_eq!(query.len(), 1);
    assert_eq!(query.first().unwrap().0, "expand");
    assert_eq!(query.first().unwrap().1, "inferenceProviderMapping");
    let body: Object = serde_json::from_slice(&prepared.http.body).unwrap();
    assert_eq!(
        body.get("model"),
        Some(&PromptJson::Text(String::from(
            "meta-llama/Llama-3-8B-Instruct"
        )))
    );
}

#[tokio::test]
async fn the_replicate_arm_creates_and_polls_with_the_strict_policy() {
    let services = Services::new(
        ScriptedHub::new(Vec::new()),
        ScriptedPost::new(Vec::new()),
        ScriptedTransport::new(vec![
            replicate_reply(200, CREATE),
            replicate_reply(200, DONE),
        ]),
    );
    let globals = empty_globals();
    let env = environment(&[]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let key = secret("ctx-key");
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context(MODEL, &mut messages, &mut options, None, Some(&key));
    let completion = dispatch(
        &selection(ChatConfigTag::Replicate),
        &mut call,
        &deps,
        &mut log,
    )
    .await
    .unwrap();
    let Some(HubCompletion::Replicate(completion)) = completion else {
        panic!("the replicate tag returns a replicate completion");
    };
    let ReplicateCompletion::Reply(reply) = *completion else {
        panic!("a non-stream call returns the reply");
    };
    assert_eq!(reply.text, "ok");
    assert_eq!(reply.model, "replicate/meta-llama/llama-2-70b-chat");
    assert_eq!(reply.created, Some(NOW_SECONDS));
    assert_eq!(reply.usage.prompt_tokens, 1);
    assert_eq!(reply.usage.total_tokens, 3);
    let requests = services.transport.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests.first().unwrap().url,
        "https://api.replicate.com/v1/models/meta-llama/llama-2-70b-chat/predictions"
    );
    assert_eq!(
        requests
            .first()
            .unwrap()
            .headers
            .get("authorization")
            .unwrap(),
        "Token ctx-key"
    );
    let sent: Object =
        serde_json::from_slice(requests.first().unwrap().body.as_deref().unwrap()).unwrap();
    assert_eq!(sent, serde_json::from_str::<Object>(PROMPT).unwrap());
    assert_eq!(requests.get(1).unwrap().url, "https://poll/1");
    assert_eq!(
        services
            .sleeper
            .delays()
            .first()
            .map(|delay| delay.to_bits()),
        Some(1.0_f64.to_bits())
    );
    assert!(state.model_call_details.get("log_event_type").is_none());
}

#[tokio::test]
async fn the_replicate_arm_uses_the_lenient_policy_for_an_async_call() {
    let services = Services::new(
        ScriptedHub::new(Vec::new()),
        ScriptedPost::new(Vec::new()),
        ScriptedTransport::new(vec![
            replicate_reply(200, CREATE),
            replicate_reply(200, r#"{"status":"succeeded","output":5}"#),
        ]),
    );
    let globals = empty_globals();
    let env = environment(&[]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = ChatContext {
        acompletion: true,
        ..chat_context(MODEL, &mut messages, &mut options, None, None)
    };
    let completion = dispatch(
        &selection(ChatConfigTag::Replicate),
        &mut call,
        &deps,
        &mut log,
    )
    .await
    .unwrap();
    let Some(HubCompletion::Replicate(completion)) = completion else {
        panic!("the replicate tag returns a replicate completion");
    };
    let ReplicateCompletion::Reply(reply) = *completion else {
        panic!("a non-stream call returns the reply");
    };
    assert_eq!(reply.text, "5");
    assert_eq!(reply.created, None);
}

#[tokio::test]
async fn the_replicate_arm_reports_a_missing_prediction_url() {
    let services = Services::new(
        ScriptedHub::new(Vec::new()),
        ScriptedPost::new(Vec::new()),
        ScriptedTransport::new(vec![replicate_reply(200, "{}")]),
    );
    let globals = empty_globals();
    let env = environment(&[]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context(MODEL, &mut messages, &mut options, None, None);
    let error = match dispatch(
        &selection(ChatConfigTag::Replicate),
        &mut call,
        &deps,
        &mut log,
    )
    .await
    {
        Ok(_) => panic!("a create reply without a poll url fails"),
        Err(error) => error,
    };
    let HubDispatchError::Replicate(failure) = error else {
        panic!("the missing url is a replicate failure");
    };
    let Some(api) = failure.api() else {
        panic!("the missing url is a Replicate API error");
    };
    assert_eq!(api.status_code, 400);
    assert!(
        api.message
            .starts_with("LiteLLM Error - prediction url is None - ")
    );
}

#[tokio::test]
async fn the_replicate_arm_logs_the_post_call_for_a_stream() {
    let services = Services::new(
        ScriptedHub::new(Vec::new()),
        ScriptedPost::new(Vec::new()),
        ScriptedTransport::new(vec![replicate_reply(200, CREATE)]),
    );
    let globals = empty_globals();
    let env = environment(&[]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let mut messages = vec![user_message()];
    let mut options = object(&[("stream", PromptJson::Bool(true))]);
    let mut call = chat_context(MODEL, &mut messages, &mut options, None, None);
    let completion = dispatch(
        &selection(ChatConfigTag::Replicate),
        &mut call,
        &deps,
        &mut log,
    )
    .await
    .unwrap();
    let Some(HubCompletion::Replicate(completion)) = completion else {
        panic!("the replicate tag returns a replicate completion");
    };
    assert!(matches!(*completion, ReplicateCompletion::Stream(_)));
    assert_eq!(
        state.model_call_details.get("log_event_type"),
        Some(&Json::Text(String::from("post_api_call")))
    );
    let Some(Json::List(input)) = state.model_call_details.get("input") else {
        panic!("the post_call keeps the messages");
    };
    assert_eq!(input.len(), 1);
}

#[tokio::test]
async fn the_predibase_arm_sends_the_generate_request_and_wraps_the_reply() {
    let services = Services::new(
        ScriptedHub::new(Vec::new()),
        ScriptedPost::new(vec![http_reply(200, r#"{"generated_text":"hello"}"#)]),
        ScriptedTransport::new(Vec::new()),
    );
    let globals = empty_globals();
    let env = environment(&[("PREDIBASE_API_KEY", "pb-env")]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let mut messages = vec![user_message()];
    let mut options = object(&[("tenant_id", PromptJson::Text(String::from("tenant-1")))]);
    let mut call = ChatContext {
        acompletion: true,
        ..chat_context(
            "meta-llama/llama-2-7b-chat",
            &mut messages,
            &mut options,
            Some("http://mock"),
            None,
        )
    };
    let completion = dispatch(
        &selection(ChatConfigTag::Predibase),
        &mut call,
        &deps,
        &mut log,
    )
    .await
    .unwrap();
    let Some(HubCompletion::Predibase(completion)) = completion else {
        panic!("the predibase tag returns a predibase completion");
    };
    let predibase::Completion::Reply(reply) = *completion else {
        panic!("a non-stream call returns the reply");
    };
    assert_eq!(reply.model, "meta-llama/llama-2-7b-chat");
    assert_eq!(reply.created, NOW_SECONDS);
    assert_eq!(
        reply
            .choices
            .first()
            .and_then(|choice| choice.content.as_deref()),
        Some("hello")
    );
    let requests = services.post.requests();
    assert_eq!(requests.len(), 1);
    let (url, sent_headers, sent_body) = requests.first().unwrap();
    assert_eq!(
        url,
        "http://mock/tenant-1/deployments/v2/llms/meta-llama/llama-2-7b-chat/generate"
    );
    assert_eq!(sent_headers.get("Authorization").unwrap(), "Bearer pb-env");
    assert_eq!(
        sent_headers.get("content-type").unwrap(),
        "application/json"
    );
    let body: Object = serde_json::from_str(sent_body).unwrap();
    assert_eq!(
        body.get("inputs"),
        Some(&PromptJson::Text(String::from("<s>[INST] hi [/INST]\n")))
    );
    assert_eq!(
        body.get("parameters"),
        Some(&PromptJson::Object(object(&[
            ("details", PromptJson::Bool(true)),
            ("max_new_tokens", number(4096)),
            ("return_full_text", PromptJson::Bool(false)),
        ])))
    );
    assert_eq!(
        state.model_call_details.get("log_event_type"),
        Some(&Json::Text(String::from("post_api_call")))
    );
}

#[tokio::test]
async fn the_predibase_arm_fails_with_the_tenant_id_text() {
    let services = Services::idle();
    let globals = empty_globals();
    let env = environment(&[("PREDIBASE_API_KEY", "pb-env")]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context("m", &mut messages, &mut options, Some("http://mock"), None);
    let error = match dispatch(
        &selection(ChatConfigTag::Predibase),
        &mut call,
        &deps,
        &mut log,
    )
    .await
    {
        Ok(_) => panic!("a missing tenant id fails the call"),
        Err(error) => error,
    };
    let HubDispatchError::PredibaseSetup(PredibaseSetupError::MissingTenantId) = &error else {
        panic!("the missing tenant id is a setup error");
    };
    assert_eq!(
        error.to_string(),
        "Missing Predibase Tenant ID - Required for making the request. Set dynamically (e.g. `completion(..tenant_id=<MY-ID>)`) or in env - `PREDIBASE_TENANT_ID`."
    );
    assert!(services.post.requests().is_empty());
}

#[tokio::test]
async fn the_predibase_arm_streams_with_the_stream_suffix_and_body_flag() {
    let services = Services::idle();
    let globals = empty_globals();
    let env = environment(&[("PREDIBASE_API_KEY", "pb-env")]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let mut messages = vec![user_message()];
    let mut options = object(&[
        ("stream", PromptJson::Bool(true)),
        ("tenant_id", PromptJson::Text(String::from("tenant-1"))),
    ]);
    let mut call = ChatContext {
        acompletion: true,
        ..chat_context("m", &mut messages, &mut options, Some("http://mock"), None)
    };
    let error = match dispatch(
        &selection(ChatConfigTag::Predibase),
        &mut call,
        &deps,
        &mut log,
    )
    .await
    {
        Ok(_) => panic!("the scripted stream transport fails"),
        Err(error) => error,
    };
    let HubDispatchError::Operation(OperationError::UnsupportedOperation { provider, .. }) = error
    else {
        panic!("the stream error comes from the transport");
    };
    assert_eq!(provider, "predibase");
    let streams = services.post.stream_requests();
    assert_eq!(streams.len(), 1);
    let (url, _, sent_body) = streams.first().unwrap();
    assert_eq!(
        url,
        "http://mock/tenant-1/deployments/v2/llms/m/generate_stream"
    );
    let body: Object = serde_json::from_str(sent_body).unwrap();
    assert_eq!(body.get("stream"), Some(&PromptJson::Bool(true)));
    let Some(PromptJson::Object(parameters)) = body.get("parameters") else {
        panic!("the request keeps its parameters");
    };
    assert!(!parameters.contains_key("stream"));
    assert!(services.post.requests().is_empty());
}

#[test]
fn the_rerank_config_takes_its_key_and_base_from_the_chains() {
    let hf_global = secret("hf-global");
    let mut globals = empty_globals();
    globals.huggingface_key = Some(&hf_global);
    let env = environment(&[
        ("HUGGINGFACE_API_KEY", "hf-api"),
        ("HF_API_BASE", "http://hf"),
    ]);
    let call_key = secret("call");
    let explicit = huggingface_rerank_config(Some(&call_key), Some("http://call"), &globals, &env)
        .expect("the rerank config builds");
    assert_eq!(text(explicit.key.as_ref()), Some(String::from("call")));
    assert_eq!(explicit.api_base.as_deref(), Some("http://call"));
    let inherited = huggingface_rerank_config(None, None, &globals, &env)
        .expect("the rerank config builds from the chains");
    assert_eq!(
        text(inherited.key.as_ref()),
        Some(String::from("hf-global"))
    );
    assert_eq!(inherited.api_base.as_deref(), Some("http://hf"));
    assert_eq!(
        inherited.get_complete_url(None, "m", None),
        "http://hf/rerank"
    );
    assert_eq!(
        inherited.get_complete_url(Some("http://call"), "m", None),
        "http://call/rerank"
    );
    let headers = inherited
        .validate_environment(EnvironmentRequest {
            headers: &Headers::new(),
            model: "m",
            api_key: None,
            optional_params: None,
            litellm_params: None,
        })
        .expect("the headers build");
    assert_eq!(headers.get("Authorization").unwrap(), "Bearer hf-global");
    let supported = inherited.supported_rerank_params("m");
    assert!(supported.contains(&String::from("query")));
}
