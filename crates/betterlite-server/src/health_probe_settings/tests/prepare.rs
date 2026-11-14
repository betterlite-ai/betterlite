use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rand::RngExt as _;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use serde_json::Number;

use crate::health_probe_settings::model_settings::HealthSettingsError;
use crate::health_probe_settings::prepare::update_params_for_health_check;
use crate::health_probe_settings::tests::support::{StubModels, object, policy};

const CHAT_MODEL: &str = "chat-model";

fn models() -> StubModels {
    let mut models = StubModels::with_mode(CHAT_MODEL, "chat");
    models
        .modes
        .insert("speech-model".to_owned(), "audio_speech".to_owned());
    models
}

fn probe(model_info: &str, litellm_params: &str, models: &StubModels) -> JsonObject {
    let mut random = StdRng::seed_from_u64(7);
    update_params_for_health_check(
        &object(model_info),
        &object(litellm_params),
        &policy(None, None),
        models,
        &mut random,
    )
    .unwrap()
}

fn max_tokens(params: &JsonObject) -> Option<i64> {
    match params.get("max_tokens") {
        Some(Json::Number(number)) => number.as_i64(),
        _ => None,
    }
}

fn text<'spec>(params: &'spec JsonObject, key: &str) -> Option<&'spec str> {
    match params.get(key) {
        Some(Json::Text(value)) => Some(value),
        _ => None,
    }
}

#[test]
fn the_deployment_params_are_never_changed() {
    let litellm_params = object(r#"{"model":"chat-model","temperature":0.2}"#);
    let original = litellm_params.clone();
    let mut random = StdRng::seed_from_u64(7);
    update_params_for_health_check(
        &object(r#"{"mode":"chat"}"#),
        &litellm_params,
        &policy(None, None),
        &models(),
        &mut random,
    )
    .unwrap();
    assert_eq!(litellm_params, original);
}

#[test]
fn health_check_params_merge_into_the_probe() {
    let params = probe(
        r#"{"mode":"chat","health_check_params":{"temperature":1,"mediaSource":"x"}}"#,
        r#"{"model":"chat-model","temperature":0.2}"#,
        &models(),
    );
    assert_eq!(
        params.get("temperature"),
        Some(&Json::Number(Number::from(1_i64)))
    );
    assert_eq!(text(&params, "mediaSource"), Some("x"));
}

#[test]
fn the_dedicated_knobs_win_over_health_check_params() {
    let params = probe(
        r#"{"mode":"chat","health_check_max_tokens":9,"health_check_params":{"max_tokens":5}}"#,
        r#"{"model":"chat-model"}"#,
        &models(),
    );
    assert_eq!(max_tokens(&params), Some(9));
}

#[test]
fn an_audio_speech_probe_defaults_the_voice_over_the_merged_params() {
    let params = probe(
        r#"{"mode":"audio_speech","health_check_params":{"voice":"echo"}}"#,
        r#"{"model":"speech-model"}"#,
        &models(),
    );
    assert_eq!(text(&params, "voice"), Some("alloy"));
}

#[test]
fn the_probe_carries_litellms_random_user_message() {
    let messages = ["Hey how's it going?", "What's 1 + 1?"];
    let mut draw = StdRng::seed_from_u64(11);
    let roll = draw.random_range(0..messages.len());
    let mut random = StdRng::seed_from_u64(11);
    let params = update_params_for_health_check(
        &object(r#"{"mode":"chat"}"#),
        &object(r#"{"model":"chat-model"}"#),
        &policy(None, None),
        &models(),
        &mut random,
    )
    .unwrap();
    let Some(Json::List(sent)) = params.get("messages") else {
        panic!("messages must be a list");
    };
    assert_eq!(sent.len(), 1);
    let Some(Json::Object(message)) = sent.first() else {
        panic!("the message must be an object");
    };
    assert_eq!(message.get("role"), Some(&Json::Text("user".to_owned())));
    assert_eq!(
        message.get("content"),
        Some(&Json::Text(messages[roll].to_owned()))
    );
}

#[rstest]
#[case::chat("chat", Some(16))]
#[case::embedding("embedding", None)]
fn max_tokens_are_added_for_chat_style_routes_only(
    #[case] mode: &str,
    #[case] expected: Option<i64>,
) {
    let mut models = models();
    models
        .modes
        .insert("other-model".to_owned(), mode.to_owned());
    let params = probe("{}", r#"{"model":"other-model"}"#, &models);
    assert_eq!(max_tokens(&params), expected);
}

#[rstest]
#[case::unresolvable("", true)]
#[case::chat("chat", true)]
#[case::completion("completion", true)]
#[case::embedding("embedding", false)]
fn reasoning_effort_reaches_chat_style_modes(#[case] mode: &str, #[case] forwarded: bool) {
    let mut models = models();
    if !mode.is_empty() {
        models
            .modes
            .insert("effort-model".to_owned(), mode.to_owned());
    }
    let params = probe(
        r#"{"health_check_reasoning_effort":"none"}"#,
        r#"{"model":"effort-model"}"#,
        &models,
    );
    assert_eq!(
        text(&params, "reasoning_effort"),
        forwarded.then_some("none")
    );
}

#[test]
fn an_audio_speech_probe_takes_the_configured_voice() {
    let params = probe(
        r#"{"mode":"audio_speech","health_check_voice":"verse"}"#,
        r#"{"model":"speech-model"}"#,
        &models(),
    );
    assert_eq!(text(&params, "voice"), Some("verse"));
}

#[test]
fn a_chat_probe_takes_no_voice() {
    let params = probe(
        r#"{"mode":"chat","health_check_voice":"verse"}"#,
        r#"{"model":"chat-model"}"#,
        &models(),
    );
    assert_eq!(params.get("voice"), None);
}

#[test]
fn health_check_model_replaces_the_probe_model() {
    let params = probe(
        r#"{"mode":"chat","health_check_model":"override-model"}"#,
        r#"{"model":"chat-model"}"#,
        &models(),
    );
    assert_eq!(text(&params, "model"), Some("override-model"));
}

#[rstest]
#[case::with_region("bedrock/us-west-2/anthropic.claude-3", "anthropic.claude-3")]
#[case::with_a_route(
    "bedrock/converse/us-west-2/anthropic.claude-3",
    "converse/anthropic.claude-3"
)]
#[case::without_a_region("bedrock/anthropic.claude-3", "anthropic.claude-3")]
#[case::a_cross_region_prefix("bedrock/us.cohere.embed-v4:0", "us.cohere.embed-v4:0")]
#[case::an_apac_prefix("bedrock/apac.amazon.nova-pro-v1:0", "apac.amazon.nova-pro-v1:0")]
fn bedrock_region_routing_is_stripped(#[case] model: &str, #[case] expected: &str) {
    let params = probe(
        r#"{"mode":"chat"}"#,
        &format!(r#"{{"model":"{model}"}}"#),
        &models(),
    );
    assert_eq!(text(&params, "model"), Some(expected));
    assert_eq!(text(&params, "custom_llm_provider"), Some("bedrock"));
}

#[test]
fn an_explicit_provider_survives_the_bedrock_rewrite() {
    let params = probe(
        r#"{"mode":"chat"}"#,
        r#"{"model":"bedrock/us-west-2/anthropic.claude-3","custom_llm_provider":"bedrock_converse"}"#,
        &models(),
    );
    assert_eq!(
        text(&params, "custom_llm_provider"),
        Some("bedrock_converse")
    );
}

#[test]
fn a_non_bedrock_model_gets_no_provider() {
    let params = probe(r#"{"mode":"chat"}"#, r#"{"model":"chat-model"}"#, &models());
    assert_eq!(text(&params, "model"), Some("chat-model"));
    assert_eq!(params.get("custom_llm_provider"), None);
}

#[test]
fn the_model_override_is_stripped_too() {
    let params = probe(
        r#"{"mode":"chat","health_check_model":"bedrock/eu-west-1/amazon.titan"}"#,
        r#"{"model":"chat-model"}"#,
        &models(),
    );
    assert_eq!(text(&params, "model"), Some("amazon.titan"));
    assert_eq!(text(&params, "custom_llm_provider"), Some("bedrock"));
}

#[test]
fn a_malformed_knob_fails_the_probe() {
    let mut random = StdRng::seed_from_u64(7);
    let error = update_params_for_health_check(
        &object(r#"{"health_check_max_tokens":"many"}"#),
        &object(r#"{"model":"chat-model"}"#),
        &policy(None, None),
        &models(),
        &mut random,
    )
    .unwrap_err();
    assert_eq!(
        error,
        HealthSettingsError::Invalid {
            field: "health_check_max_tokens",
            expected: "an integer",
        }
    );
}
