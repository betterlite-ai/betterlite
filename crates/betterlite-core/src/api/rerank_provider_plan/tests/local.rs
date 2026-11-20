use super::*;

#[test]
fn rerank_local_and_hosted_credentials() {
    let ops = FakeOps::default();
    hosted_vllm_takes_the_call_values(&ops);
    hosted_vllm_takes_the_environment_values(&ops);
    hosted_vllm_without_a_base_raises(&ops);
    deepinfra_takes_the_call_key_and_environment_base(&ops);
    deepinfra_without_a_base_raises(&ops);
    fireworks_takes_the_dynamic_values(&ops);
    fireworks_takes_the_call_key(&ops);
    fireworks_takes_the_first_environment_name(&ops);
    fireworks_falls_through_an_empty_name(&ops);
    fireworks_takes_the_third_name(&ops);
    fireworks_takes_the_token_without_a_base(&ops);
    voyage_takes_the_second_name(&ops);
    voyage_takes_the_environment_base(&ops);
    voyage_without_a_key_still_dispatches(&ops);
}

fn local_settings() -> RerankSettings {
    let mut settings = defaults();
    settings.api_key = Some(SecretString::from("global-key"));
    settings.api_base = Some(String::from("https://global.test/v1"));
    settings
}

fn hosted_vllm_takes_the_call_values(ops: &FakeOps) {
    let hosted = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("hosted_vllm".to_owned()),
        None,
        None,
    );
    let mut call = rerank_call("hosted_vllm");
    call.kwargs = with(&[
        ("api_key", text("call-key")),
        ("api_base", text("https://call.test/v1")),
    ]);
    run_one(
        &hosted,
        ops,
        &local_settings(),
        &[("HOSTED_VLLM_API_KEY", "env-key")],
        call,
    )
    .expect("hosted vllm takes the call values");
    let send = last(ops);
    assert_eq!(send.config, SelectedConfig::HostedVllm);
    assert_eq!(send.api_key.as_deref(), Some("call-key"));
    assert_eq!(send.api_base.as_deref(), Some("https://call.test/v1"));
}

fn hosted_vllm_takes_the_environment_values(ops: &FakeOps) {
    let hosted = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("hosted_vllm".to_owned()),
        None,
        None,
    );
    run_one(
        &hosted,
        ops,
        &local_settings(),
        &[
            ("HOSTED_VLLM_API_KEY", "env-key"),
            ("HOSTED_VLLM_API_BASE", "https://vllm-env.test"),
        ],
        rerank_call("hosted_vllm"),
    )
    .expect("hosted vllm takes the environment values");
    let send = last(ops);
    assert_eq!(send.api_key.as_deref(), Some("env-key"));
    assert_eq!(send.api_base.as_deref(), Some("https://vllm-env.test"));
}

fn hosted_vllm_without_a_base_raises(ops: &FakeOps) {
    let hosted = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("hosted_vllm".to_owned()),
        None,
        None,
    );
    let sent_before = ops.sends().len();
    let error = run_one(&hosted, ops, &defaults(), &[], rerank_call("hosted_vllm"))
        .expect_err("hosted vllm needs a base");
    assert_eq!(
        error.message,
        "api_base must be provided for Hosted VLLM rerank. Set in call or via HOSTED_VLLM_API_BASE env var."
    );
    assert_eq!(ops.sends().len(), sent_before);
}

fn deepinfra_takes_the_call_key_and_environment_base(ops: &FakeOps) {
    let deepinfra = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("deepinfra".to_owned()),
        None,
        None,
    );
    let mut call = rerank_call("deepinfra");
    call.kwargs = with(&[("api_key", text("call-key"))]);
    run_one(
        &deepinfra,
        ops,
        &local_settings(),
        &[
            ("DEEPINFRA_API_KEY", "env-key"),
            ("DEEPINFRA_API_BASE", "https://di-env.test"),
        ],
        call,
    )
    .expect("deepinfra dispatches");
    let send = last(ops);
    assert_eq!(send.config, SelectedConfig::Deepinfra);
    assert_eq!(send.api_key.as_deref(), Some("call-key"));
    assert_eq!(send.api_base.as_deref(), Some("https://di-env.test"));
}

fn deepinfra_without_a_base_raises(ops: &FakeOps) {
    let deepinfra = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("deepinfra".to_owned()),
        None,
        None,
    );
    let sent_before = ops.sends().len();
    let error = run_one(&deepinfra, ops, &defaults(), &[], rerank_call("deepinfra"))
        .expect_err("deepinfra needs a base");
    assert_eq!(
        error.message,
        "api_base must be provided for Deepinfra rerank. Set in call or via DEEPINFRA_API_BASE env var."
    );
    assert_eq!(ops.sends().len(), sent_before);
}

fn fireworks_takes_the_dynamic_values(ops: &FakeOps) {
    let fireworks = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("fireworks_ai".to_owned()),
        Some("dynamic-key"),
        Some("https://dynamic.test"),
    );
    run_one(
        &fireworks,
        ops,
        &local_settings(),
        &[],
        rerank_call("fireworks_ai"),
    )
    .expect("fireworks takes the dynamic values");
    let send = last(ops);
    assert_eq!(send.config, SelectedConfig::FireworksAi);
    assert_eq!(send.api_key.as_deref(), Some("dynamic-key"));
    assert_eq!(send.api_base.as_deref(), Some("https://dynamic.test"));
}

fn fireworks_takes_the_call_key(ops: &FakeOps) {
    let anonymous = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("fireworks_ai".to_owned()),
        None,
        None,
    );
    let mut call_key = rerank_call("fireworks_ai");
    call_key.kwargs = with(&[("api_key", text("call-key"))]);
    run_one(
        &anonymous,
        ops,
        &defaults(),
        &[("FIREWORKS_API_KEY", "env-first")],
        call_key,
    )
    .expect("fireworks takes the call key");
    assert_eq!(last(ops).api_key.as_deref(), Some("call-key"));
}

fn fireworks_takes_the_first_environment_name(ops: &FakeOps) {
    let anonymous = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("fireworks_ai".to_owned()),
        None,
        None,
    );
    run_one(
        &anonymous,
        ops,
        &defaults(),
        &[
            ("FIREWORKS_API_KEY", "env-first"),
            ("FIREWORKS_AI_API_KEY", "second-key"),
        ],
        rerank_call("fireworks_ai"),
    )
    .expect("fireworks takes the first environment name");
    assert_eq!(last(ops).api_key.as_deref(), Some("env-first"));
}

fn fireworks_falls_through_an_empty_name(ops: &FakeOps) {
    let anonymous = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("fireworks_ai".to_owned()),
        None,
        None,
    );
    run_one(
        &anonymous,
        ops,
        &defaults(),
        &[
            ("FIREWORKS_API_KEY", ""),
            ("FIREWORKS_AI_API_KEY", "second-key"),
            ("FIREWORKSAI_API_KEY", "third-key"),
        ],
        rerank_call("fireworks_ai"),
    )
    .expect("fireworks falls through the empty name");
    assert_eq!(last(ops).api_key.as_deref(), Some("second-key"));
}

fn fireworks_takes_the_third_name(ops: &FakeOps) {
    let anonymous = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("fireworks_ai".to_owned()),
        None,
        None,
    );
    run_one(
        &anonymous,
        ops,
        &defaults(),
        &[
            ("FIREWORKSAI_API_KEY", "third-key"),
            ("FIREWORKS_AI_TOKEN", "token-key"),
        ],
        rerank_call("fireworks_ai"),
    )
    .expect("fireworks takes the third name");
    assert_eq!(last(ops).api_key.as_deref(), Some("third-key"));
}

fn fireworks_takes_the_token_without_a_base(ops: &FakeOps) {
    let anonymous = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("fireworks_ai".to_owned()),
        None,
        None,
    );
    run_one(
        &anonymous,
        ops,
        &defaults(),
        &[("FIREWORKS_AI_TOKEN", "token-key")],
        rerank_call("fireworks_ai"),
    )
    .expect("fireworks takes the token");
    assert_eq!(last(ops).api_key.as_deref(), Some("token-key"));
    assert_eq!(
        last(ops).api_base,
        None,
        "fireworks without a base sends none"
    );
}

fn voyage_takes_the_second_name(ops: &FakeOps) {
    let voyage = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("voyage".to_owned()),
        None,
        None,
    );
    let mut voyage_settings = defaults();
    voyage_settings.environment.voyage_ai_api_key = Some(SecretString::from("second-key"));
    run_one(&voyage, ops, &voyage_settings, &[], rerank_call("voyage"))
        .expect("voyage takes the second name");
    let send = last(ops);
    assert_eq!(send.config, SelectedConfig::Voyage);
    assert_eq!(send.api_key.as_deref(), Some("second-key"));
}

fn voyage_takes_the_environment_base(ops: &FakeOps) {
    let voyage = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("voyage".to_owned()),
        None,
        None,
    );
    let mut voyage_base = defaults();
    voyage_base.environment.voyage_api_base = Some(String::from("https://voyage-env.test"));
    run_one(&voyage, ops, &voyage_base, &[], rerank_call("voyage"))
        .expect("voyage takes the environment base");
    assert_eq!(
        last(ops).api_base.as_deref(),
        Some("https://voyage-env.test")
    );
}

fn voyage_without_a_key_still_dispatches(ops: &FakeOps) {
    let voyage = FakeIdentity::new(
        "rerank-v1",
        ProviderIdentity::Custom("voyage".to_owned()),
        None,
        None,
    );
    // no branch adds a missing-key error of its own
    run_one(&voyage, ops, &defaults(), &[], rerank_call("voyage"))
        .expect("voyage without any key still dispatches");
    assert_eq!(last(ops).api_key, None);
}
