use betterlite_error::envelope::WireField;

use super::*;

fn run_one(
    identity: &FakeIdentity,
    ops: &FakeOps,
    settings: &RerankSettings,
    env: &[(&str, &str)],
    call: RerankCall,
) -> Result<RerankResponse, super::super::plan::RerankException> {
    let mut logging = logging_state();
    dispatch(
        identity,
        ops,
        settings,
        &environment(env),
        call,
        &mut logging,
    )
}

fn last(ops: &FakeOps) -> SendSeen {
    ops.sends()
        .into_iter()
        .next_back()
        .expect("the call reached the provider")
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn head(pairs: &[(&str, &str)]) -> Json {
    Json::Object(
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), text(value)))
            .collect(),
    )
}

#[test]
fn rerank_cohere_foundry_infinity() {
    let ops = FakeOps::default();
    let mut settings = defaults();
    settings.api_key = Some(SecretString::from("global-key"));
    settings.api_base = Some(String::from("https://global.test/v1"));
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        Some("dynamic-key"),
        Some("https://dynamic.test/v1"),
    );
    let mut call = call("cohere");
    call.kwargs = with(&[
        ("api_key", text("call-key")),
        ("api_base", text("https://call.test/v1")),
        ("headers", head(&[("x-call", "c")])),
    ]);
    run_one(&identity, &ops, &settings, &[], call).expect("cohere dispatches");
    let send = last(&ops);
    assert_eq!(send.provider, "cohere");
    assert_eq!(send.api_key.as_deref(), Some("dynamic-key"));
    assert_eq!(send.api_base.as_deref(), Some("https://dynamic.test/v1"));
    assert_eq!(send.headers.get("x-call").map(String::as_str), Some("c"));
    // the selector reads the call's api_base, not the resolved one
    assert_eq!(send.config, SelectedConfig::CohereV2);

    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        None,
        None,
    );
    let mut settings = defaults();
    settings.api_base = Some(String::from("https://global.test/v1"));
    let mut call = call("cohere");
    call.kwargs = with(&[("api_key", text("call-key"))]);
    run_one(&identity, &ops, &settings, &[], call).expect("the call key dispatches");
    let send = last(&ops);
    assert_eq!(send.api_key.as_deref(), Some("call-key"));
    assert_eq!(send.api_base.as_deref(), Some("https://global.test/v1"));
    assert!(send.headers.is_empty());

    let mut settings = defaults();
    settings.headers = Some(BTreeMap::from([(
        String::from("x-global"),
        String::from("g"),
    )]));
    run_one(&identity, &ops, &settings, &[], call("cohere")).expect("the global headers dispatch");
    assert_eq!(
        last(&ops).headers.get("x-global").map(String::as_str),
        Some("g")
    );

    let mut settings = defaults();
    settings.api_key = Some(SecretString::from("global-key"));
    run_one(
        &identity,
        &ops,
        &settings,
        &[("COHERE_API_BASE", "https://env.test")],
        call("cohere"),
    )
    .expect("the environment base dispatches");
    assert_eq!(last(&ops).api_base.as_deref(), Some("https://env.test"));

    run_one(&identity, &ops, &defaults(), &[], call("cohere"))
        .expect("the default base dispatches");
    assert_eq!(
        last(&ops).api_base.as_deref(),
        Some("https://api.cohere.com")
    );

    let proxy = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("litellm_proxy".to_owned()),
        None,
        None,
    );
    run_one(&proxy, &ops, &defaults(), &[], call("litellm_proxy")).expect("the proxy dispatches");
    assert_eq!(last(&ops).provider, "litellm_proxy");
    assert_eq!(
        last(&ops).api_base.as_deref(),
        Some("https://api.cohere.com")
    );

    let azure = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("azure_ai".to_owned()),
        Some("dyn"),
        Some("https://dynamic.test"),
    );
    let mut settings = defaults();
    settings.api_base = Some(String::from("https://global.test"));
    let mut call = call("azure_ai");
    call.kwargs = with(&[
        ("api_key", text("call-key")),
        ("api_base", text("https://call.test")),
    ]);
    run_one(&azure, &ops, &settings, &[], call).expect("azure dispatches");
    let send = last(&ops);
    assert_eq!(send.provider, "azure_ai");
    assert_eq!(send.api_key.as_deref(), Some("dyn"));
    assert_eq!(send.api_base.as_deref(), Some("https://dynamic.test"));

    let azure = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("azure_ai".to_owned()),
        None,
        None,
    );
    let mut call = call("azure_ai");
    call.kwargs = with(&[
        ("api_key", text("call-key")),
        ("api_base", text("https://call.test")),
    ]);
    run_one(&azure, &ops, &settings, &[], call).expect("azure takes the call values");
    assert_eq!(last(&ops).api_key.as_deref(), Some("call-key"));
    assert_eq!(last(&ops).api_base.as_deref(), Some("https://call.test"));

    run_one(&azure, &ops, &settings, &[], call("azure_ai")).expect("azure takes the global base");
    assert_eq!(last(&ops).api_key, None);
    assert_eq!(last(&ops).api_base.as_deref(), Some("https://global.test"));

    run_one(
        &azure,
        &ops,
        &defaults(),
        &[("AZURE_AI_API_BASE", "https://env.test")],
        call("azure_ai"),
    )
    .expect("azure takes the environment base");
    assert_eq!(last(&ops).api_base.as_deref(), Some("https://env.test"));
    run_one(&azure, &ops, &defaults(), &[], call("azure_ai"))
        .expect("azure without any base still dispatches");
    assert_eq!(last(&ops).api_base, None);

    let infinity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("infinity".to_owned()),
        None,
        Some("https://dynamic.test"),
    );
    let mut settings = defaults();
    settings.api_base = Some(String::from("https://global.test"));
    settings.api_key = Some(SecretString::from("global-key"));
    let mut call = call("infinity");
    call.kwargs = with(&[("api_base", text("https://call.test"))]);
    run_one(&infinity, &ops, &settings, &[], call).expect("infinity dispatches");
    let send = last(&ops);
    assert_eq!(send.config, SelectedConfig::Infinity);
    assert_eq!(send.api_base.as_deref(), Some("https://dynamic.test"));
    // the branch computed a global key and never sent it
    assert_eq!(send.api_key, None);

    let infinity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("infinity".to_owned()),
        None,
        None,
    );
    run_one(
        &infinity,
        &ops,
        &settings,
        &[("INFINITY_API_BASE", "https://env.test")],
        call("infinity"),
    )
    .expect("infinity takes the environment base");
    assert_eq!(last(&ops).api_base.as_deref(), Some("https://env.test"));

    let sent_before = ops.sends().len();
    let error = run_one(&infinity, &ops, &defaults(), &[], call("infinity"))
        .expect_err("a missing infinity base raises");
    assert_eq!(
        error.message,
        "Invalid api base. api_base=None. Set in call or via `INFINITY_API_BASE` env var."
    );
    assert_eq!(ops.sends().len(), sent_before);
}

#[test]
fn rerank_together_jina_nvidia() {
    let ops = FakeOps::default();
    let mut settings = defaults();
    settings.api_key = Some(SecretString::from("global-key"));
    settings.togetherai_api_key = Some(SecretString::from("settings-key"));
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("together_ai".to_owned()),
        Some("dynamic-key"),
        Some("https://dynamic.test/v1"),
    );
    run_one(
        &identity,
        &ops,
        &settings,
        &[("TOGETHERAI_API_KEY", "env-key")],
        call("together_ai"),
    )
    .expect("together dispatches");
    let send = last(&ops);
    assert_eq!(send.api_key.as_deref(), Some("dynamic-key"));
    assert_eq!(send.api_base.as_deref(), Some("https://dynamic.test/v1"));

    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("together_ai".to_owned()),
        None,
        None,
    );
    let mut call = call("together_ai");
    call.kwargs = with(&[("api_key", text("call-key"))]);
    run_one(
        &identity,
        &ops,
        &settings,
        &[("TOGETHERAI_API_KEY", "env-key")],
        call,
    )
    .expect("together takes the call key");
    assert_eq!(last(&ops).api_key.as_deref(), Some("call-key"));

    run_one(
        &identity,
        &ops,
        &settings,
        &[("TOGETHERAI_API_KEY", "env-key")],
        call("together_ai"),
    )
    .expect("together takes the settings key");
    assert_eq!(last(&ops).api_key.as_deref(), Some("settings-key"));

    let mut environment_settings = settings.clone();
    environment_settings.togetherai_api_key = None;
    run_one(
        &identity,
        &ops,
        &environment_settings,
        &[("TOGETHERAI_API_KEY", "env-key")],
        call("together_ai"),
    )
    .expect("together takes the environment key");
    assert_eq!(last(&ops).api_key.as_deref(), Some("env-key"));

    run_one(
        &identity,
        &ops,
        &environment_settings,
        &[],
        call("together_ai"),
    )
    .expect("together takes the global key");
    assert_eq!(last(&ops).api_key.as_deref(), Some("global-key"));
    assert_eq!(
        last(&ops).api_base.as_deref(),
        Some("https://api.together.ai/v1")
    );

    let mut settings = defaults();
    let sent_before = ops.sends().len();
    let error = run_one(&identity, &ops, &settings, &[], call("together_ai"))
        .expect_err("a missing together key raises");
    assert_eq!(
        error.message,
        "TogetherAI API key is required, please set 'TOGETHERAI_API_KEY' in your environment"
    );
    assert_eq!(ops.sends().len(), sent_before);

    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("together_ai".to_owned()),
        Some("dynamic-key"),
        Some("https://dynamic.test/v1"),
    );
    let mut call = call("together_ai");
    call.top_n = Some(1_i64);
    call.rank_fields = Some(vec![String::from("text")]);
    call.max_chunks_per_doc = Some(2_i64);
    call.max_tokens_per_doc = Some(3_i64);
    call.return_documents = ReturnDocuments::Value(false);
    call.kwargs = with(&[("instruction", text("brief")), ("timeout", text("4"))]);
    run_one(&identity, &ops, &settings, &[], call).expect("together forwards the options");
    let send = last(&ops);
    assert_eq!(send.top_n, Some(1_i64));
    assert_eq!(send.rank_fields, Some(vec![String::from("text")]));
    assert_eq!(send.max_chunks_per_doc, Some(2_i64));
    assert_eq!(send.max_tokens_per_doc, Some(3_i64));
    assert_eq!(send.return_documents, ReturnDocuments::Value(false));
    assert_eq!(send.instruction.as_deref(), Some("brief"));
    assert_eq!(send.timeout_seconds, Some(4.0_f64));

    let jina = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("jina_ai".to_owned()),
        None,
        None,
    );
    let mut call = call("jina_ai");
    call.kwargs = with(&[("api_key", text("call-key"))]);
    let sent_before = ops.sends().len();
    let error =
        run_one(&jina, &ops, &defaults(), &[], call).expect_err("a missing jina key raises");
    assert_eq!(
        error.message,
        "Jina AI API key is required, please set 'JINA_AI_API_KEY' in your environment"
    );
    assert_eq!(ops.sends().len(), sent_before);

    let jina = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("jina_ai".to_owned()),
        Some("dynamic-key"),
        None,
    );
    run_one(
        &jina,
        &ops,
        &defaults(),
        &[("BEDROCK_API_BASE", "https://bedrock-env.test")],
        call("jina_ai"),
    )
    .expect("jina takes the bedrock base");
    let send = last(&ops);
    assert_eq!(send.config, SelectedConfig::JinaAi);
    assert_eq!(send.api_base.as_deref(), Some("https://bedrock-env.test"));

    let nvidia = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("nvidia_nim".to_owned()),
        None,
        Some("https://dynamic.test"),
    );
    let sent_before = ops.sends().len();
    let error = run_one(&nvidia, &ops, &defaults(), &[], call("nvidia_nim"))
        .expect_err("a missing nvidia key raises");
    assert_eq!(
        error.message,
        "Nvidia NIM API key is required, please set 'NVIDIA_NIM_API_KEY' in your environment"
    );
    assert_eq!(ops.sends().len(), sent_before);

    let nvidia = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("nvidia_nim".to_owned()),
        Some("dynamic-key"),
        Some("https://dynamic.test"),
    );
    let mut settings = defaults();
    settings.api_base = Some(String::from("https://global.test"));
    let mut call = call("nvidia_nim");
    call.kwargs = with(&[("api_base", text("https://call.test"))]);
    run_one(&nvidia, &ops, &settings, &[], call).expect("nvidia takes the call base");
    let send = last(&ops);
    assert_eq!(send.config, SelectedConfig::NvidiaRetrieval);
    assert_eq!(send.api_base.as_deref(), Some("https://call.test"));
    assert_eq!(send.api_key.as_deref(), Some("dynamic-key"));

    run_one(
        &nvidia,
        &ops,
        &settings,
        &[("NVIDIA_NIM_API_BASE", "https://nim-env.test")],
        call("nvidia_nim"),
    )
    .expect("nvidia takes the environment base");
    assert_eq!(last(&ops).api_base.as_deref(), Some("https://nim-env.test"));

    run_one(&nvidia, &ops, &settings, &[], call("nvidia_nim"))
        .expect("nvidia takes its default base");
    assert_eq!(
        last(&ops).api_base.as_deref(),
        Some("https://ai.api.nvidia.com")
    );

    let ranking = FakeIdentity::new(
        "ranking/nvidia/rerank-qa",
        ProviderIdentity::Custom("nvidia_nim".to_owned()),
        Some("dynamic-key"),
        None,
    );
    run_one(&ranking, &ops, &settings, &[], call("nvidia_nim"))
        .expect("the ranking call dispatches");
    assert_eq!(last(&ops).config, SelectedConfig::NvidiaRanking);
}

#[test]
fn rerank_local_and_hosted_credentials() {
    let ops = FakeOps::default();
    let mut settings = defaults();
    settings.api_key = Some(SecretString::from("global-key"));
    settings.api_base = Some(String::from("https://global.test/v1"));

    let hosted = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("hosted_vllm".to_owned()),
        None,
        None,
    );
    let mut call = call("hosted_vllm");
    call.kwargs = with(&[
        ("api_key", text("call-key")),
        ("api_base", text("https://call.test/v1")),
    ]);
    run_one(
        &hosted,
        &ops,
        &settings,
        &[("HOSTED_VLLM_API_KEY", "env-key")],
        call,
    )
    .expect("hosted vllm takes the call values");
    let send = last(&ops);
    assert_eq!(send.config, SelectedConfig::HostedVllm);
    assert_eq!(send.api_key.as_deref(), Some("call-key"));
    assert_eq!(send.api_base.as_deref(), Some("https://call.test/v1"));

    run_one(
        &hosted,
        &ops,
        &settings,
        &[
            ("HOSTED_VLLM_API_KEY", "env-key"),
            ("HOSTED_VLLM_API_BASE", "https://vllm-env.test"),
        ],
        call("hosted_vllm"),
    )
    .expect("hosted vllm takes the environment values");
    let send = last(&ops);
    assert_eq!(send.api_key.as_deref(), Some("env-key"));
    assert_eq!(send.api_base.as_deref(), Some("https://vllm-env.test"));

    let sent_before = ops.sends().len();
    let error = run_one(&hosted, &ops, &defaults(), &[], call("hosted_vllm"))
        .expect_err("hosted vllm needs a base");
    assert_eq!(
        error.message,
        "api_base must be provided for Hosted VLLM rerank. Set in call or via HOSTED_VLLM_API_BASE env var."
    );
    assert_eq!(ops.sends().len(), sent_before);

    let deepinfra = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("deepinfra".to_owned()),
        None,
        None,
    );
    let mut call = call("deepinfra");
    call.kwargs = with(&[("api_key", text("call-key"))]);
    run_one(
        &deepinfra,
        &ops,
        &settings,
        &[
            ("DEEPINFRA_API_KEY", "env-key"),
            ("DEEPINFRA_API_BASE", "https://di-env.test"),
        ],
        call,
    )
    .expect("deepinfra dispatches");
    let send = last(&ops);
    assert_eq!(send.config, SelectedConfig::Deepinfra);
    assert_eq!(send.api_key.as_deref(), Some("call-key"));
    assert_eq!(send.api_base.as_deref(), Some("https://di-env.test"));

    let sent_before = ops.sends().len();
    let error = run_one(&deepinfra, &ops, &defaults(), &[], call("deepinfra"))
        .expect_err("deepinfra needs a base");
    assert_eq!(
        error.message,
        "api_base must be provided for Deepinfra rerank. Set in call or via DEEPINFRA_API_BASE env var."
    );
    assert_eq!(ops.sends().len(), sent_before);

    let fireworks = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("fireworks_ai".to_owned()),
        Some("dynamic-key"),
        Some("https://dynamic.test"),
    );
    run_one(&fireworks, &ops, &settings, &[], call("fireworks_ai"))
        .expect("fireworks takes the dynamic values");
    let send = last(&ops);
    assert_eq!(send.config, SelectedConfig::FireworksAi);
    assert_eq!(send.api_key.as_deref(), Some("dynamic-key"));
    assert_eq!(send.api_base.as_deref(), Some("https://dynamic.test"));

    let anonymous = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("fireworks_ai".to_owned()),
        None,
        None,
    );
    let mut call_key = call("fireworks_ai");
    call_key.kwargs = with(&[("api_key", text("call-key"))]);
    run_one(
        &anonymous,
        &ops,
        &defaults(),
        &[("FIREWORKS_API_KEY", "env-first")],
        call_key,
    )
    .expect("fireworks takes the call key");
    assert_eq!(last(&ops).api_key.as_deref(), Some("call-key"));

    run_one(
        &anonymous,
        &ops,
        &defaults(),
        &[
            ("FIREWORKS_API_KEY", "env-first"),
            ("FIREWORKS_AI_API_KEY", "second-key"),
        ],
        call("fireworks_ai"),
    )
    .expect("fireworks takes the first environment name");
    assert_eq!(last(&ops).api_key.as_deref(), Some("env-first"));

    run_one(
        &anonymous,
        &ops,
        &defaults(),
        &[
            ("FIREWORKS_API_KEY", ""),
            ("FIREWORKS_AI_API_KEY", "second-key"),
            ("FIREWORKSAI_API_KEY", "third-key"),
        ],
        call("fireworks_ai"),
    )
    .expect("fireworks falls through the empty name");
    assert_eq!(last(&ops).api_key.as_deref(), Some("second-key"));

    run_one(
        &anonymous,
        &ops,
        &defaults(),
        &[
            ("FIREWORKSAI_API_KEY", "third-key"),
            ("FIREWORKS_AI_TOKEN", "token-key"),
        ],
        call("fireworks_ai"),
    )
    .expect("fireworks takes the third name");
    assert_eq!(last(&ops).api_key.as_deref(), Some("third-key"));

    run_one(
        &anonymous,
        &ops,
        &defaults(),
        &[("FIREWORKS_AI_TOKEN", "token-key")],
        call("fireworks_ai"),
    )
    .expect("fireworks takes the token");
    assert_eq!(last(&ops).api_key.as_deref(), Some("token-key"));
    assert_eq!(
        last(&ops).api_base,
        None,
        "fireworks without a base sends none"
    );

    let voyage = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("voyage".to_owned()),
        None,
        None,
    );
    let mut voyage_settings = defaults();
    voyage_settings.environment.voyage_ai_api_key = Some(SecretString::from("second-key"));
    run_one(&voyage, &ops, &voyage_settings, &[], call("voyage"))
        .expect("voyage takes the second name");
    let send = last(&ops);
    assert_eq!(send.config, SelectedConfig::Voyage);
    assert_eq!(send.api_key.as_deref(), Some("second-key"));

    let mut voyage_base = defaults();
    voyage_base.environment.voyage_api_base = Some(String::from("https://voyage-env.test"));
    run_one(&voyage, &ops, &voyage_base, &[], call("voyage"))
        .expect("voyage takes the environment base");
    assert_eq!(
        last(&ops).api_base.as_deref(),
        Some("https://voyage-env.test")
    );

    // no branch adds a missing-key error of its own
    run_one(&voyage, &ops, &defaults(), &[], call("voyage"))
        .expect("voyage without any key still dispatches");
    assert_eq!(last(&ops).api_key, None);
}

#[test]
fn rerank_bedrock_watsonx_and_generic() {
    let ops = FakeOps::default();
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("bedrock".to_owned()),
        Some("dynamic-key"),
        Some("https://dynamic.test"),
    );
    let mut call = call("bedrock");
    call.kwargs = with(&[
        ("headers", head(&[("x", "a")])),
        ("extra_headers", head(&[("x", "b"), ("y", "c")])),
        ("timeout", text("3")),
    ]);
    run_one(&identity, &ops, &defaults(), &[], call).expect("bedrock dispatches");
    let send = last(&ops);
    assert_eq!(send.provider, "bedrock");
    assert_eq!(send.headers.get("x").map(String::as_str), Some("b"));
    assert_eq!(send.headers.get("y").map(String::as_str), Some("c"));
    assert_eq!(send.api_base.as_deref(), Some("https://dynamic.test"));
    assert_eq!(send.timeout_seconds, Some(3.0_f64));
    assert_eq!(
        send.generic_params.get("extra_headers"),
        Some(&head(&[("x", "b"), ("y", "c")]))
    );

    let bedrock = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("bedrock".to_owned()),
        None,
        None,
    );
    let mut settings = defaults();
    settings.api_base = Some(String::from("https://global.test"));
    run_one(&bedrock, &ops, &settings, &[], call("bedrock"))
        .expect("bedrock takes the global base");
    assert_eq!(last(&ops).api_base.as_deref(), Some("https://global.test"));
    run_one(
        &bedrock,
        &ops,
        &defaults(),
        &[("BEDROCK_API_BASE", "https://bedrock-env.test")],
        call("bedrock"),
    )
    .expect("bedrock takes the environment base");
    assert_eq!(
        last(&ops).api_base.as_deref(),
        Some("https://bedrock-env.test")
    );

    let watsonx = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("watsonx".to_owned()),
        None,
        None,
    );
    let credentials = Json::Object(IndexMap::from([
        (String::from("apikey"), text("wx-key")),
        (String::from("url"), text("https://watsonx.test")),
        (String::from("token"), text("")),
    ]));
    let mut call = call("watsonx");
    call.kwargs = with(&[("wx_credentials", credentials)]);
    run_one(&watsonx, &ops, &defaults(), &[], call).expect("watsonx dispatches");
    let send = last(&ops);
    assert_eq!(send.config, SelectedConfig::Watsonx);
    assert_eq!(send.api_key.as_deref(), Some("wx-key"));
    assert_eq!(send.api_base.as_deref(), Some("https://watsonx.test"));
    assert_eq!(send.optional_params.get("token"), Some(&text("")));

    let credentials = Json::Object(IndexMap::from([
        (String::from("apikey"), text("wx-key")),
        (String::from("url"), text("https://watsonx.test")),
    ]));
    let mut call = call("watsonx");
    call.kwargs = with(&[("wx_credentials", credentials)]);
    run_one(&watsonx, &ops, &defaults(), &[], call).expect("watsonx without a token dispatches");
    assert_eq!(last(&ops).optional_params.get("token"), None);

    let generic = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("openai".to_owned()),
        None,
        None,
    );
    let sent_before = ops.sends().len();
    let error = run_one(&generic, &ops, &defaults(), &[], call("openai"))
        .expect_err("an unknown provider with a cohere config raises");
    assert_eq!(error.message, "Unsupported provider: openai");
    assert_eq!(ops.sends().len(), sent_before);

    let chat = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("cohere_chat".to_owned()),
        None,
        None,
    );
    let error = run_one(&chat, &ops, &defaults(), &[], call("cohere_chat"))
        .expect_err("the cohere chat provider keeps the cohere config");
    assert_eq!(error.message, "Unsupported provider: cohere_chat");

    let failing = FakeOps::failing_send(429, "slow down");
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        Some("k"),
        Some("https://dynamic.test"),
    );
    let error = run_one(&identity, &failing, &defaults(), &[], call("cohere"))
        .expect_err("the provider failure raises");
    assert_eq!(error.message, "slow down");
    let metadata = error.class.metadata();
    assert_eq!(metadata.status_code, 429);
    assert_eq!(
        metadata.error_type,
        WireField::Text(String::from("throttling_error"))
    );
    assert_eq!(metadata.code, "429");
    assert_eq!(metadata.openai_code.as_deref(), Some("429"));
    assert_eq!(failing.sends().len(), 1);

    let missing = FakeIdentity::missing();
    let error = run_one(&missing, &ops, &defaults(), &[], call("cohere"))
        .expect_err("an unresolvable provider raises");
    assert!(error.message.starts_with("LLM Provider NOT provided."));
}

#[test]
fn an_empty_environment_snapshot_falls_through() {
    let ops = FakeOps::default();
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        None,
        None,
    );
    let mut settings = defaults();
    settings.environment.cohere_api_base = Some(String::new());
    run_one(&identity, &ops, &settings, &[], call("cohere"))
        .expect("the empty environment base falls through");
    assert_eq!(
        last(&ops).api_base.as_deref(),
        Some("https://api.cohere.com")
    );
}
