#![cfg(test)]

use betterlite_models::decisions::Json;
use chrono::{TimeZone as _, Timelike as _, Utc};
use rstest::rstest;
use rust_decimal::Decimal;

use crate::langfuse_observations::identity::{Timestamp, resolve_observation_id};

use super::support::{absent, attribute, fixture, json, object_field, payload, run, text, value};

fn usage(usage: &str) -> Json {
    value(&format!(r#"{{"usage": {usage}}}"#))
}

fn datetime() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 1, 2, 3)
        .single()
        .expect("a valid date")
        .with_nanosecond(456_789_000)
        .expect("a valid nanosecond")
}

#[rstest]
#[case::metadata_name(Some("custom-name"), Some("prod-key"), "custom-name")]
#[case::key_alias(None, Some("prod-key"), "litellm:prod-key")]
#[case::call_type(None, None, "litellm-completion")]
fn the_generation_name_falls_back_in_order(
    #[case] generation_name: Option<&str>,
    #[case] key_alias: Option<&str>,
    #[case] expected: &str,
) {
    let mut case = fixture();
    if let Some(alias) = key_alias {
        case.inputs
            .set_metadata("user_api_key_alias", Json::Text(alias.to_owned()));
    }
    if let Some(name) = generation_name {
        case.inputs
            .set_metadata("generation_name", Json::Text(name.to_owned()));
    }
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(generation.name.to_string(), expected);
}

#[test]
fn a_metadata_generation_id_becomes_the_span_id() {
    let mut case = fixture();
    case.inputs
        .set_metadata("generation_id", Json::Text("0102030405060708".to_owned()));
    let outcome = run(case.assemble());
    assert_eq!(outcome.generation_id.as_deref(), Some("0102030405060708"));
}

#[test]
fn the_response_id_and_start_time_build_the_generation_id() {
    let mut case = fixture();
    case.inputs.response = Some(value(r#"{"id": "chatcmpl-9"}"#));
    case.inputs.start_time = Some(Timestamp::DateTime(datetime()));
    let outcome = run(case.assemble());
    let expected = resolve_observation_id(Some("time-01-02-03-456789_chatcmpl-9"))
        .expect("a span id")
        .to_string();
    assert_eq!(outcome.generation_id, Some(expected));
}

#[test]
fn a_response_without_an_id_carries_no_generation_id() {
    let mut case = fixture();
    case.inputs.response = Some(value(r#"{"object": "chat.completion"}"#));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        outcome.generation_id,
        Some(generation.span_context.span_id().to_string())
    );
}

#[test]
fn usage_details_reduce_the_input_by_the_cache_reads() {
    let mut case = fixture();
    case.inputs.response = Some(usage(
        r#"{"prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
            "cache_read_input_tokens": 30, "cache_creation_input_tokens": 10}"#,
    ));
    case.inputs.response_cost = Some(Decimal::new(25, 2));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        json(generation, "langfuse.observation.usage_details"),
        Some(value(
            r#"{"input": 70, "output": 20, "total": 120,
                "cache_creation_input_tokens": 10, "cache_read_input_tokens": 30}"#
        ))
    );
    assert_eq!(
        json(generation, "langfuse.observation.cost_details"),
        Some(value(r#"{"total": 0.25}"#))
    );
}

#[test]
fn cached_token_details_override_the_top_level_cache_read() {
    let mut case = fixture();
    case.inputs.response = Some(usage(
        r#"{"prompt_tokens": 100, "completion_tokens": 5, "total_tokens": 105,
            "prompt_tokens_details": {"cached_tokens": 40}}"#,
    ));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        json(generation, "langfuse.observation.usage_details"),
        Some(value(
            r#"{"input": 60, "output": 5, "total": 105,
                "cache_creation_input_tokens": 0, "cache_read_input_tokens": 40}"#
        ))
    );
}

#[test]
fn null_token_counts_become_zero() {
    let mut case = fixture();
    case.inputs.response = Some(usage(
        r#"{"prompt_tokens": null, "completion_tokens": null, "total_tokens": null}"#,
    ));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        json(generation, "langfuse.observation.usage_details"),
        Some(value(
            r#"{"input": 0, "output": 0, "total": 0,
                "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0}"#
        ))
    );
}

#[test]
fn an_empty_usage_object_carries_no_usage_or_cost_details() {
    let mut case = fixture();
    case.inputs.response = Some(usage("{}"));
    case.inputs.response_cost = Some(Decimal::new(5, 1));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert!(absent(generation, "langfuse.observation.usage_details"));
    assert!(absent(generation, "langfuse.observation.cost_details"));
}

#[test]
fn cost_details_need_a_response_cost() {
    let mut case = fixture();
    case.inputs.response = Some(usage(
        r#"{"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}"#,
    ));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert!(absent(generation, "langfuse.observation.cost_details"));
}

#[rstest]
#[case::bedrock(Some("bedrock"), "claude-3", "bedrock/claude-3")]
#[case::already_prefixed(Some("bedrock"), "bedrock/claude-3", "bedrock/claude-3")]
#[case::other_provider(Some("openai"), "gpt-4o", "gpt-4o")]
#[case::no_provider(None, "gpt-4o", "gpt-4o")]
fn the_model_name_carries_the_bedrock_prefix(
    #[case] provider: Option<&'static str>,
    #[case] model: &'static str,
    #[case] expected: &str,
) {
    let mut case = fixture();
    case.inputs.provider = provider;
    case.inputs.model = model;
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        text(generation, "langfuse.observation.model.name").as_deref(),
        Some(expected)
    );
}

#[test]
fn a_deployment_metadata_name_wins_over_the_model() {
    let mut case = fixture();
    case.inputs.provider = Some("bedrock");
    case.inputs
        .set_metadata("deployment", Json::Text("anthropic/claude-3".to_owned()));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        text(generation, "langfuse.observation.model.name").as_deref(),
        Some("anthropic/claude-3")
    );
}

#[test]
fn a_deployment_name_without_a_slash_is_ignored() {
    let mut case = fixture();
    case.inputs.provider = Some("bedrock");
    case.inputs
        .set_metadata("deployment", Json::Text("claude-3-deploy".to_owned()));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_ne!(
        text(generation, "langfuse.observation.model.name").as_deref(),
        Some("claude-3-deploy")
    );
}

#[test]
fn an_empty_model_gets_no_bedrock_prefix() {
    let mut case = fixture();
    case.inputs.provider = Some("bedrock");
    case.inputs.model = "";
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_ne!(
        text(generation, "langfuse.observation.model.name").as_deref(),
        Some("bedrock/")
    );
}

#[test]
fn the_system_fingerprint_reaches_the_model_parameters() {
    let mut case = fixture();
    case.inputs
        .optional_params
        .insert("temperature".to_owned(), value("0.5"));
    case.inputs.response = Some(value(r#"{"system_fingerprint": "fp-1"}"#));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        json(generation, "langfuse.observation.model.parameters"),
        Some(value(
            r#"{"temperature": 0.5, "system_fingerprint": "fp-1"}"#
        ))
    );
}

#[test]
fn error_levels_carry_the_status_message() {
    let mut case = fixture();
    case.inputs.level = "ERROR";
    case.inputs.output = Some(Json::Text("boom".to_owned()));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        text(generation, "langfuse.observation.status_message").as_deref(),
        Some("boom")
    );
    assert_eq!(
        text(generation, "langfuse.observation.level").as_deref(),
        Some("ERROR")
    );
}

#[test]
fn other_levels_carry_no_status_message() {
    let mut case = fixture();
    case.inputs.output = Some(Json::Text("fine".to_owned()));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert!(absent(generation, "langfuse.observation.status_message"));
}

#[test]
fn a_user_prompt_links_the_generation() {
    let mut case = fixture();
    case.inputs.set_metadata(
        "prompt",
        value(
            r#"{"type": "text", "name": "greeting", "version": 3, "prompt": "hello",
                "config": null, "labels": [], "tags": []}"#,
        ),
    );
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        text(generation, "langfuse.observation.prompt.name").as_deref(),
        Some("greeting")
    );
    assert_eq!(
        attribute(generation, "langfuse.observation.prompt.version"),
        Some(opentelemetry::Value::I64(3))
    );
}

#[test]
fn enrichments_carry_the_api_base_and_provider_locations() {
    let mut case = fixture();
    case.inputs.litellm_params = value(r#"{"api_base": "https://api.example"}"#);
    case.inputs.vertex_location = Some("us-central1");
    case.inputs.aws_region_name = Some("eu-west-1");
    case.inputs.cache_hit = Some(true);
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        text(generation, "langfuse.observation.metadata.api_base").as_deref(),
        Some("https://api.example")
    );
    assert_eq!(
        text(generation, "langfuse.observation.metadata.vertex_location").as_deref(),
        Some("us-central1")
    );
    assert_eq!(
        text(generation, "langfuse.observation.metadata.aws_region_name").as_deref(),
        Some("eu-west-1")
    );
    assert_eq!(
        attribute(generation, "langfuse.observation.metadata.cache_hit"),
        Some(opentelemetry::Value::Bool(true))
    );
}

#[test]
fn the_generation_metadata_carries_the_cost_hidden_params_and_ids() {
    let mut case = fixture();
    case.inputs.payload = Some(payload("{}"));
    case.inputs.response = Some(value(r#"{"id": "chatcmpl-1"}"#));
    case.inputs.litellm_call_id = Some("call-1");
    case.inputs.response_cost = Some(Decimal::new(5, 1));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        text(
            generation,
            "langfuse.observation.metadata.litellm_response_cost"
        )
        .as_deref(),
        Some("0.5")
    );
    assert_eq!(
        text(generation, "langfuse.observation.metadata.litellm_call_id").as_deref(),
        Some("call-1")
    );
    assert_eq!(
        text(generation, "langfuse.observation.metadata.response_id").as_deref(),
        Some("chatcmpl-1")
    );
    let hidden =
        json(generation, "langfuse.observation.metadata.hidden_params").expect("the hidden params");
    assert_eq!(
        object_field(&hidden, "cache_key"),
        Some(Json::Text("hidden-1".to_owned()))
    );
}

#[test]
fn the_payload_metadata_reaches_the_generation_metadata() {
    let mut case = fixture();
    case.inputs.payload = Some(payload(
        r#"{"team_alias": "core", "requester_metadata": {"tier": "gold"}}"#,
    ));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        text(generation, "langfuse.observation.metadata.team_alias").as_deref(),
        Some("core")
    );
    assert_eq!(
        json(
            generation,
            "langfuse.observation.metadata.requester_metadata"
        ),
        Some(value(r#"{"tier": "gold"}"#))
    );
}
