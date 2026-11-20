use betterlite_error::envelope::WireField;

use super::*;

mod local;

fn run_one(
    identity: &FakeIdentity,
    ops: &FakeOps,
    settings: &RerankSettings,
    env: &[(&str, &str)],
    call: RerankCall,
) -> Result<RerankResponse, RerankException> {
    let mut logging = logging_state();
    Case::new(ops, settings.clone(), environment(env)).dispatch(identity, call, &mut logging)
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
    cohere_dynamic_credentials_win(&ops);
    cohere_call_key_and_settings_base(&ops);
    cohere_global_headers_are_merged(&ops);
    cohere_environment_base_wins_over_default(&ops);
    cohere_default_base_is_used(&ops);
    litellm_proxy_keeps_the_cohere_default_base(&ops);
    azure_ai_prefers_its_own_credentials(&ops);
    azure_ai_takes_the_call_values(&ops);
    azure_ai_takes_the_global_base(&ops);
    azure_ai_takes_the_environment_base_then_none(&ops);
    infinity_omits_the_computed_global_key(&ops);
    infinity_takes_the_environment_base(&ops);
    infinity_without_a_base_raises(&ops);
}

fn cohere_dynamic_credentials_win(ops: &FakeOps) {
    let mut settings = defaults();
    settings.api_key = Some(SecretString::from("global-key"));
    settings.api_base = Some(String::from("https://global.test/v1"));
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        Some("dynamic-key"),
        Some("https://dynamic.test/v1"),
    );
    let mut call = rerank_call("cohere");
    call.kwargs = with(&[
        ("api_key", text("call-key")),
        ("api_base", text("https://call.test/v1")),
        ("headers", head(&[("x-call", "c")])),
    ]);
    run_one(&identity, ops, &settings, &[], call).expect("cohere dispatches");
    let send = last(ops);
    assert_eq!(send.provider, "cohere");
    assert_eq!(send.api_key.as_deref(), Some("dynamic-key"));
    assert_eq!(send.api_base.as_deref(), Some("https://dynamic.test/v1"));
    assert_eq!(send.headers.get("x-call").map(String::as_str), Some("c"));
    // the selector reads the call's api_base, not the resolved one
    assert_eq!(send.config, SelectedConfig::CohereV2);
}

fn cohere_call_key_and_settings_base(ops: &FakeOps) {
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        None,
        None,
    );
    let mut settings = defaults();
    settings.api_base = Some(String::from("https://global.test/v1"));
    let mut call = rerank_call("cohere");
    call.kwargs = with(&[("api_key", text("call-key"))]);
    run_one(&identity, ops, &settings, &[], call).expect("the call key dispatches");
    let send = last(ops);
    assert_eq!(send.api_key.as_deref(), Some("call-key"));
    assert_eq!(send.api_base.as_deref(), Some("https://global.test/v1"));
    assert!(send.headers.is_empty());
}

fn cohere_global_headers_are_merged(ops: &FakeOps) {
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        None,
        None,
    );
    let mut settings = defaults();
    settings.headers = Some(BTreeMap::from([(
        String::from("x-global"),
        String::from("g"),
    )]));
    run_one(&identity, ops, &settings, &[], rerank_call("cohere"))
        .expect("the global headers dispatch");
    assert_eq!(
        last(ops).headers.get("x-global").map(String::as_str),
        Some("g")
    );
}

fn cohere_environment_base_wins_over_default(ops: &FakeOps) {
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        None,
        None,
    );
    let mut settings = defaults();
    settings.api_key = Some(SecretString::from("global-key"));
    run_one(
        &identity,
        ops,
        &settings,
        &[("COHERE_API_BASE", "https://env.test")],
        rerank_call("cohere"),
    )
    .expect("the environment base dispatches");
    assert_eq!(last(ops).api_base.as_deref(), Some("https://env.test"));
}

fn cohere_default_base_is_used(ops: &FakeOps) {
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        None,
        None,
    );
    run_one(&identity, ops, &defaults(), &[], rerank_call("cohere"))
        .expect("the default base dispatches");
    assert_eq!(
        last(ops).api_base.as_deref(),
        Some("https://api.cohere.com")
    );
}

fn litellm_proxy_keeps_the_cohere_default_base(ops: &FakeOps) {
    let proxy = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("litellm_proxy".to_owned()),
        None,
        None,
    );
    run_one(&proxy, ops, &defaults(), &[], rerank_call("litellm_proxy"))
        .expect("the proxy dispatches");
    assert_eq!(last(ops).provider, "litellm_proxy");
    assert_eq!(
        last(ops).api_base.as_deref(),
        Some("https://api.cohere.com")
    );
}

fn azure_ai_prefers_its_own_credentials(ops: &FakeOps) {
    let azure = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("azure_ai".to_owned()),
        Some("dyn"),
        Some("https://dynamic.test"),
    );
    let mut settings = defaults();
    settings.api_base = Some(String::from("https://global.test"));
    let mut call = rerank_call("azure_ai");
    call.kwargs = with(&[
        ("api_key", text("call-key")),
        ("api_base", text("https://call.test")),
    ]);
    run_one(&azure, ops, &settings, &[], call).expect("azure dispatches");
    let send = last(ops);
    assert_eq!(send.provider, "azure_ai");
    assert_eq!(send.api_key.as_deref(), Some("dyn"));
    assert_eq!(send.api_base.as_deref(), Some("https://dynamic.test"));
}

fn azure_ai_takes_the_call_values(ops: &FakeOps) {
    let azure = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("azure_ai".to_owned()),
        None,
        None,
    );
    let mut settings = defaults();
    settings.api_base = Some(String::from("https://global.test"));
    let mut call = rerank_call("azure_ai");
    call.kwargs = with(&[
        ("api_key", text("call-key")),
        ("api_base", text("https://call.test")),
    ]);
    run_one(&azure, ops, &settings, &[], call).expect("azure takes the call values");
    assert_eq!(last(ops).api_key.as_deref(), Some("call-key"));
    assert_eq!(last(ops).api_base.as_deref(), Some("https://call.test"));
}

fn azure_ai_takes_the_global_base(ops: &FakeOps) {
    let azure = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("azure_ai".to_owned()),
        None,
        None,
    );
    let mut settings = defaults();
    settings.api_base = Some(String::from("https://global.test"));
    run_one(&azure, ops, &settings, &[], rerank_call("azure_ai"))
        .expect("azure takes the global base");
    assert_eq!(last(ops).api_key, None);
    assert_eq!(last(ops).api_base.as_deref(), Some("https://global.test"));
}

fn azure_ai_takes_the_environment_base_then_none(ops: &FakeOps) {
    let azure = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("azure_ai".to_owned()),
        None,
        None,
    );
    run_one(
        &azure,
        ops,
        &defaults(),
        &[("AZURE_AI_API_BASE", "https://env.test")],
        rerank_call("azure_ai"),
    )
    .expect("azure takes the environment base");
    assert_eq!(last(ops).api_base.as_deref(), Some("https://env.test"));
    run_one(&azure, ops, &defaults(), &[], rerank_call("azure_ai"))
        .expect("azure without any base still dispatches");
    assert_eq!(last(ops).api_base, None);
}

fn infinity_omits_the_computed_global_key(ops: &FakeOps) {
    let infinity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("infinity".to_owned()),
        None,
        Some("https://dynamic.test"),
    );
    let mut settings = defaults();
    settings.api_base = Some(String::from("https://global.test"));
    settings.api_key = Some(SecretString::from("global-key"));
    let mut call = rerank_call("infinity");
    call.kwargs = with(&[("api_base", text("https://call.test"))]);
    run_one(&infinity, ops, &settings, &[], call).expect("infinity dispatches");
    let send = last(ops);
    assert_eq!(send.config, SelectedConfig::Infinity);
    assert_eq!(send.api_base.as_deref(), Some("https://dynamic.test"));
    // the branch computed a global key and never sent it
    assert_eq!(send.api_key, None);
}

fn infinity_takes_the_environment_base(ops: &FakeOps) {
    let infinity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("infinity".to_owned()),
        None,
        None,
    );
    run_one(
        &infinity,
        ops,
        &defaults(),
        &[("INFINITY_API_BASE", "https://env.test")],
        rerank_call("infinity"),
    )
    .expect("infinity takes the environment base");
    assert_eq!(last(ops).api_base.as_deref(), Some("https://env.test"));
}

fn infinity_without_a_base_raises(ops: &FakeOps) {
    let infinity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("infinity".to_owned()),
        None,
        None,
    );
    let sent_before = ops.sends().len();
    let error = run_one(&infinity, ops, &defaults(), &[], rerank_call("infinity"))
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
    together_dynamic_credentials_win(&ops);
    together_takes_the_call_key(&ops);
    together_takes_the_settings_key(&ops);
    together_takes_the_environment_key(&ops);
    together_takes_the_global_key_and_default_base(&ops);
    together_without_a_key_raises(&ops);
    together_forwards_the_options(&ops);
    jina_without_a_key_raises(&ops);
    jina_takes_the_bedrock_base(&ops);
    nvidia_without_a_key_raises(&ops);
    nvidia_takes_the_call_base(&ops);
    nvidia_takes_the_environment_base(&ops);
    nvidia_takes_the_default_base(&ops);
    nvidia_ranking_model_selects_the_ranking_client(&ops);
}

fn together_settings() -> RerankSettings {
    let mut settings = defaults();
    settings.api_key = Some(SecretString::from("global-key"));
    settings.togetherai_api_key = Some(SecretString::from("settings-key"));
    settings
}

fn together_dynamic_credentials_win(ops: &FakeOps) {
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("together_ai".to_owned()),
        Some("dynamic-key"),
        Some("https://dynamic.test/v1"),
    );
    run_one(
        &identity,
        ops,
        &together_settings(),
        &[("TOGETHERAI_API_KEY", "env-key")],
        rerank_call("together_ai"),
    )
    .expect("together dispatches");
    let send = last(ops);
    assert_eq!(send.api_key.as_deref(), Some("dynamic-key"));
    assert_eq!(send.api_base.as_deref(), Some("https://dynamic.test/v1"));
}

fn together_takes_the_call_key(ops: &FakeOps) {
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("together_ai".to_owned()),
        None,
        None,
    );
    let mut call = rerank_call("together_ai");
    call.kwargs = with(&[("api_key", text("call-key"))]);
    run_one(
        &identity,
        ops,
        &together_settings(),
        &[("TOGETHERAI_API_KEY", "env-key")],
        call,
    )
    .expect("together takes the call key");
    assert_eq!(last(ops).api_key.as_deref(), Some("call-key"));
}

fn together_takes_the_settings_key(ops: &FakeOps) {
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("together_ai".to_owned()),
        None,
        None,
    );
    run_one(
        &identity,
        ops,
        &together_settings(),
        &[("TOGETHERAI_API_KEY", "env-key")],
        rerank_call("together_ai"),
    )
    .expect("together takes the settings key");
    assert_eq!(last(ops).api_key.as_deref(), Some("settings-key"));
}

fn together_takes_the_environment_key(ops: &FakeOps) {
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("together_ai".to_owned()),
        None,
        None,
    );
    let mut environment_settings = together_settings();
    environment_settings.togetherai_api_key = None;
    run_one(
        &identity,
        ops,
        &environment_settings,
        &[("TOGETHERAI_API_KEY", "env-key")],
        rerank_call("together_ai"),
    )
    .expect("together takes the environment key");
    assert_eq!(last(ops).api_key.as_deref(), Some("env-key"));
}

fn together_takes_the_global_key_and_default_base(ops: &FakeOps) {
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("together_ai".to_owned()),
        None,
        None,
    );
    let mut environment_settings = together_settings();
    environment_settings.togetherai_api_key = None;
    run_one(
        &identity,
        ops,
        &environment_settings,
        &[],
        rerank_call("together_ai"),
    )
    .expect("together takes the global key");
    assert_eq!(last(ops).api_key.as_deref(), Some("global-key"));
    assert_eq!(
        last(ops).api_base.as_deref(),
        Some("https://api.together.ai/v1")
    );
}

fn together_without_a_key_raises(ops: &FakeOps) {
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("together_ai".to_owned()),
        None,
        None,
    );
    let sent_before = ops.sends().len();
    let error = run_one(&identity, ops, &defaults(), &[], rerank_call("together_ai"))
        .expect_err("a missing together key raises");
    assert_eq!(
        error.message,
        "TogetherAI API key is required, please set 'TOGETHERAI_API_KEY' in your environment"
    );
    assert_eq!(ops.sends().len(), sent_before);
}

fn together_forwards_the_options(ops: &FakeOps) {
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("together_ai".to_owned()),
        Some("dynamic-key"),
        Some("https://dynamic.test/v1"),
    );
    let mut call = rerank_call("together_ai");
    call.top_n = Some(1_i64);
    call.rank_fields = Some(vec![String::from("text")]);
    call.max_chunks_per_doc = Some(2_i64);
    call.max_tokens_per_doc = Some(3_i64);
    call.return_documents = ReturnDocuments::Value(false);
    call.kwargs = with(&[("instruction", text("brief")), ("timeout", text("4"))]);
    run_one(&identity, ops, &defaults(), &[], call).expect("together forwards the options");
    let send = last(ops);
    assert_eq!(send.top_n, Some(1_i64));
    assert_eq!(send.rank_fields, Some(vec![String::from("text")]));
    assert_eq!(send.max_chunks_per_doc, Some(2_i64));
    assert_eq!(send.max_tokens_per_doc, Some(3_i64));
    assert_eq!(send.return_documents, ReturnDocuments::Value(false));
    assert_eq!(send.instruction.as_deref(), Some("brief"));
    assert_eq!(send.timeout_seconds, Some(4.0_f64));
}

fn jina_without_a_key_raises(ops: &FakeOps) {
    let jina = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("jina_ai".to_owned()),
        None,
        None,
    );
    let mut call = rerank_call("jina_ai");
    call.kwargs = with(&[("api_key", text("call-key"))]);
    let sent_before = ops.sends().len();
    let error = run_one(&jina, ops, &defaults(), &[], call).expect_err("a missing jina key raises");
    assert_eq!(
        error.message,
        "Jina AI API key is required, please set 'JINA_AI_API_KEY' in your environment"
    );
    assert_eq!(ops.sends().len(), sent_before);
}

fn jina_takes_the_bedrock_base(ops: &FakeOps) {
    let jina = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("jina_ai".to_owned()),
        Some("dynamic-key"),
        None,
    );
    run_one(
        &jina,
        ops,
        &defaults(),
        &[("BEDROCK_API_BASE", "https://bedrock-env.test")],
        rerank_call("jina_ai"),
    )
    .expect("jina takes the bedrock base");
    let send = last(ops);
    assert_eq!(send.config, SelectedConfig::JinaAi);
    assert_eq!(send.api_base.as_deref(), Some("https://bedrock-env.test"));
}

fn nvidia_without_a_key_raises(ops: &FakeOps) {
    let nvidia = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("nvidia_nim".to_owned()),
        None,
        Some("https://dynamic.test"),
    );
    let sent_before = ops.sends().len();
    let error = run_one(&nvidia, ops, &defaults(), &[], rerank_call("nvidia_nim"))
        .expect_err("a missing nvidia key raises");
    assert_eq!(
        error.message,
        "Nvidia NIM API key is required, please set 'NVIDIA_NIM_API_KEY' in your environment"
    );
    assert_eq!(ops.sends().len(), sent_before);
}

fn nvidia_takes_the_call_base(ops: &FakeOps) {
    let nvidia = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("nvidia_nim".to_owned()),
        Some("dynamic-key"),
        Some("https://dynamic.test"),
    );
    let mut settings = defaults();
    settings.api_base = Some(String::from("https://global.test"));
    let mut call = rerank_call("nvidia_nim");
    call.kwargs = with(&[("api_base", text("https://call.test"))]);
    run_one(&nvidia, ops, &settings, &[], call).expect("nvidia takes the call base");
    let send = last(ops);
    assert_eq!(send.config, SelectedConfig::NvidiaRetrieval);
    assert_eq!(send.api_base.as_deref(), Some("https://call.test"));
    assert_eq!(send.api_key.as_deref(), Some("dynamic-key"));
}

fn nvidia_takes_the_environment_base(ops: &FakeOps) {
    let nvidia = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("nvidia_nim".to_owned()),
        Some("dynamic-key"),
        Some("https://dynamic.test"),
    );
    let mut settings = defaults();
    settings.api_base = Some(String::from("https://global.test"));
    run_one(
        &nvidia,
        ops,
        &settings,
        &[("NVIDIA_NIM_API_BASE", "https://nim-env.test")],
        rerank_call("nvidia_nim"),
    )
    .expect("nvidia takes the environment base");
    assert_eq!(last(ops).api_base.as_deref(), Some("https://nim-env.test"));
}

fn nvidia_takes_the_default_base(ops: &FakeOps) {
    let nvidia = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("nvidia_nim".to_owned()),
        Some("dynamic-key"),
        Some("https://dynamic.test"),
    );
    let mut settings = defaults();
    settings.api_base = Some(String::from("https://global.test"));
    run_one(&nvidia, ops, &settings, &[], rerank_call("nvidia_nim"))
        .expect("nvidia takes its default base");
    assert_eq!(
        last(ops).api_base.as_deref(),
        Some("https://ai.api.nvidia.com")
    );
}

fn nvidia_ranking_model_selects_the_ranking_client(ops: &FakeOps) {
    let ranking = FakeIdentity::new(
        "ranking/nvidia/rerank-qa",
        ProviderIdentity::Custom("nvidia_nim".to_owned()),
        Some("dynamic-key"),
        None,
    );
    let mut settings = defaults();
    settings.api_base = Some(String::from("https://global.test"));
    run_one(&ranking, ops, &settings, &[], rerank_call("nvidia_nim"))
        .expect("the ranking call dispatches");
    assert_eq!(last(ops).config, SelectedConfig::NvidiaRanking);
}

#[test]
fn rerank_bedrock_watsonx_and_generic() {
    let ops = FakeOps::default();
    bedrock_merges_extra_headers_and_timeout(&ops);
    bedrock_takes_the_global_base(&ops);
    bedrock_takes_the_environment_base(&ops);
    watsonx_reads_the_credentials_object(&ops);
    watsonx_without_a_token_sends_none(&ops);
    an_unknown_provider_raises(&ops);
    the_cohere_chat_provider_keeps_the_cohere_config(&ops);
    a_provider_failure_is_converted();
    an_unresolvable_provider_raises(&ops);
}

fn bedrock_merges_extra_headers_and_timeout(ops: &FakeOps) {
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("bedrock".to_owned()),
        Some("dynamic-key"),
        Some("https://dynamic.test"),
    );
    let mut call = rerank_call("bedrock");
    call.kwargs = with(&[
        ("headers", head(&[("x", "a")])),
        ("extra_headers", head(&[("x", "b"), ("y", "c")])),
        ("timeout", text("3")),
    ]);
    run_one(&identity, ops, &defaults(), &[], call).expect("bedrock dispatches");
    let send = last(ops);
    assert_eq!(send.provider, "bedrock");
    assert_eq!(send.headers.get("x").map(String::as_str), Some("b"));
    assert_eq!(send.headers.get("y").map(String::as_str), Some("c"));
    assert_eq!(send.api_base.as_deref(), Some("https://dynamic.test"));
    assert_eq!(send.timeout_seconds, Some(3.0_f64));
    assert_eq!(
        send.generic_params.get("extra_headers"),
        Some(&head(&[("x", "b"), ("y", "c")]))
    );
}

fn bedrock_takes_the_global_base(ops: &FakeOps) {
    let bedrock = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("bedrock".to_owned()),
        None,
        None,
    );
    let mut settings = defaults();
    settings.api_base = Some(String::from("https://global.test"));
    run_one(&bedrock, ops, &settings, &[], rerank_call("bedrock"))
        .expect("bedrock takes the global base");
    assert_eq!(last(ops).api_base.as_deref(), Some("https://global.test"));
}

fn bedrock_takes_the_environment_base(ops: &FakeOps) {
    let bedrock = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("bedrock".to_owned()),
        None,
        None,
    );
    run_one(
        &bedrock,
        ops,
        &defaults(),
        &[("BEDROCK_API_BASE", "https://bedrock-env.test")],
        rerank_call("bedrock"),
    )
    .expect("bedrock takes the environment base");
    assert_eq!(
        last(ops).api_base.as_deref(),
        Some("https://bedrock-env.test")
    );
}

fn watsonx_reads_the_credentials_object(ops: &FakeOps) {
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
    let mut call = rerank_call("watsonx");
    call.kwargs = with(&[("wx_credentials", credentials)]);
    run_one(&watsonx, ops, &defaults(), &[], call).expect("watsonx dispatches");
    let send = last(ops);
    assert_eq!(send.config, SelectedConfig::Watsonx);
    assert_eq!(send.api_key.as_deref(), Some("wx-key"));
    assert_eq!(send.api_base.as_deref(), Some("https://watsonx.test"));
    assert_eq!(send.optional_params.get("token"), Some(&text("")));
}

fn watsonx_without_a_token_sends_none(ops: &FakeOps) {
    let watsonx = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("watsonx".to_owned()),
        None,
        None,
    );
    let credentials = Json::Object(IndexMap::from([
        (String::from("apikey"), text("wx-key")),
        (String::from("url"), text("https://watsonx.test")),
    ]));
    let mut call = rerank_call("watsonx");
    call.kwargs = with(&[("wx_credentials", credentials)]);
    run_one(&watsonx, ops, &defaults(), &[], call).expect("watsonx without a token dispatches");
    assert_eq!(last(ops).optional_params.get("token"), None);
}

fn an_unknown_provider_raises(ops: &FakeOps) {
    let generic = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("openai".to_owned()),
        None,
        None,
    );
    let sent_before = ops.sends().len();
    let error = run_one(&generic, ops, &defaults(), &[], rerank_call("openai"))
        .expect_err("an unknown provider with a cohere config raises");
    assert_eq!(error.message, "Unsupported provider: openai");
    assert_eq!(ops.sends().len(), sent_before);
}

fn the_cohere_chat_provider_keeps_the_cohere_config(ops: &FakeOps) {
    let chat = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("cohere_chat".to_owned()),
        None,
        None,
    );
    let error = run_one(&chat, ops, &defaults(), &[], rerank_call("cohere_chat"))
        .expect_err("the cohere chat provider keeps the cohere config");
    assert_eq!(error.message, "Unsupported provider: cohere_chat");
}

fn a_provider_failure_is_converted() {
    let failing = FakeOps::failing_send(429, "slow down");
    let identity = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Known(ProviderId::Cohere),
        Some("k"),
        Some("https://dynamic.test"),
    );
    let error = run_one(&identity, &failing, &defaults(), &[], rerank_call("cohere"))
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
}

fn an_unresolvable_provider_raises(ops: &FakeOps) {
    let missing = FakeIdentity::missing();
    let error = run_one(&missing, ops, &defaults(), &[], rerank_call("cohere"))
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
    run_one(&identity, &ops, &settings, &[], rerank_call("cohere"))
        .expect("the empty environment base falls through");
    assert_eq!(
        last(&ops).api_base.as_deref(),
        Some("https://api.cohere.com")
    );
}
