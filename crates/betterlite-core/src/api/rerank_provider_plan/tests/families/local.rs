use betterlite_config::api::rerank_provider_plan::RerankSettings;
use betterlite_models::resolution::ProviderIdentity;
use secrecy::SecretString;

use super::super::{FakeIdentity, FakeOps, SelectedConfig, defaults, rerank_call, with};
use super::{Scenario, last, raises_without_sending, run_one, text};

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

fn local_scenario(provider: &'static str) -> Scenario<'static> {
    Scenario::new(provider).with_settings(local_settings())
}

fn sent_key_base(scenario: &Scenario<'_>, ops: &FakeOps, key: &str, base: &str) {
    let send = scenario.send(ops);
    assert_eq!(send.api_key.as_deref(), Some(key));
    assert_eq!(send.api_base.as_deref(), Some(base));
}

fn hosted_vllm_takes_the_call_values(ops: &FakeOps) {
    let scenario = local_scenario("hosted_vllm")
        .with_env(&[("HOSTED_VLLM_API_KEY", "env-key")])
        .with_kwargs(&[
            ("api_key", text("call-key")),
            ("api_base", text("https://call.test/v1")),
        ]);
    sent_key_base(&scenario, ops, "call-key", "https://call.test/v1");
}

fn hosted_vllm_takes_the_environment_values(ops: &FakeOps) {
    let scenario = local_scenario("hosted_vllm")
        .with_env(&[("HOSTED_VLLM_API_KEY", "env-key")])
        .with_env(&[("HOSTED_VLLM_API_BASE", "https://vllm-env.test")]);
    sent_key_base(&scenario, ops, "env-key", "https://vllm-env.test");
}

fn hosted_vllm_without_a_base_raises(ops: &FakeOps) {
    raises_without_sending(
        ops,
        &Scenario::new("hosted_vllm"),
        "api_base must be provided for Hosted VLLM rerank. Set in call or via HOSTED_VLLM_API_BASE env var.",
    );
}

fn deepinfra_takes_the_call_key_and_environment_base(ops: &FakeOps) {
    let scenario = local_scenario("deepinfra")
        .with_env(&[("DEEPINFRA_API_KEY", "env-key")])
        .with_env(&[("DEEPINFRA_API_BASE", "https://di-env.test")])
        .with_kwargs(&[("api_key", text("call-key"))]);
    sent_key_base(&scenario, ops, "call-key", "https://di-env.test");
}

fn deepinfra_without_a_base_raises(ops: &FakeOps) {
    raises_without_sending(
        ops,
        &Scenario::new("deepinfra"),
        "api_base must be provided for Deepinfra rerank. Set in call or via DEEPINFRA_API_BASE env var.",
    );
}

fn fireworks_takes_the_dynamic_values(ops: &FakeOps) {
    let scenario = local_scenario("fireworks_ai")
        .with_dynamic(Some("dynamic-key"), Some("https://dynamic.test"));
    sent_key_base(&scenario, ops, "dynamic-key", "https://dynamic.test");
    assert_eq!(last(ops).config, SelectedConfig::FireworksAi);
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
    let scenario = Scenario::new("fireworks_ai").with_env(&[
        ("FIREWORKS_API_KEY", "env-first"),
        ("FIREWORKS_AI_API_KEY", "second-key"),
    ]);
    assert_eq!(scenario.send(ops).api_key.as_deref(), Some("env-first"));
}

fn fireworks_falls_through_an_empty_name(ops: &FakeOps) {
    let scenario = Scenario::new("fireworks_ai").with_env(&[
        ("FIREWORKS_API_KEY", ""),
        ("FIREWORKS_AI_API_KEY", "second-key"),
        ("FIREWORKSAI_API_KEY", "third-key"),
    ]);
    assert_eq!(scenario.send(ops).api_key.as_deref(), Some("second-key"));
}

fn fireworks_takes_the_third_name(ops: &FakeOps) {
    let scenario = Scenario::new("fireworks_ai").with_env(&[
        ("FIREWORKSAI_API_KEY", "third-key"),
        ("FIREWORKS_AI_TOKEN", "token-key"),
    ]);
    assert_eq!(scenario.send(ops).api_key.as_deref(), Some("third-key"));
}

fn fireworks_takes_the_token_without_a_base(ops: &FakeOps) {
    let scenario = Scenario::new("fireworks_ai").with_env(&[("FIREWORKS_AI_TOKEN", "token-key")]);
    let send = scenario.send(ops);
    assert_eq!(send.api_key.as_deref(), Some("token-key"));
    assert_eq!(send.api_base, None, "fireworks without a base sends none");
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
