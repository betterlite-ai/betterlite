#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::langfuse_assembly::metadata;
use crate::langfuse_assembly::{AssemblyInput, AssemblySettings};

use super::support::{
    Inputs, absent, attribute, fixture, json, payload, run, string_array, text, upper, value,
};

fn cleaned(inputs: &Inputs) -> metadata::Cleaned {
    metadata::clean(&AssemblyInput {
        trace: &inputs.trace(),
        settings: &inputs.settings(),
    })
}

#[test]
fn denied_steering_keys_never_reach_the_clean_metadata() {
    let mut inputs = Inputs::default();
    inputs.set_metadata("headers", value(r#"{"authorization": "secret"}"#));
    inputs.set_metadata("endpoint", Json::Text("https://api".to_owned()));
    inputs.set_metadata("caching_groups", value("[]"));
    inputs.set_metadata("previous_models", value(r#"["gpt-3"]"#));
    inputs.set_metadata("custom", Json::Text("kept".to_owned()));
    let clean = cleaned(&inputs);
    assert_eq!(
        clean.fields.get("custom"),
        Some(&Json::Text("kept".to_owned()))
    );
    assert!(!clean.fields.contains_key("headers"));
    assert!(!clean.fields.contains_key("endpoint"));
    assert!(!clean.fields.contains_key("caching_groups"));
    assert!(!clean.fields.contains_key("previous_models"));
}

#[test]
fn the_payload_prompt_management_metadata_rides_along() {
    let mut case = fixture();
    case.inputs.payload = Some(payload(
        r#"{"prompt_management_metadata": {"prompt_id": "p-1", "prompt_integration": "langfuse"}}"#,
    ));
    let clean = cleaned(&case.inputs);
    assert_eq!(
        clean.fields.get("prompt_management_metadata"),
        Some(&value(
            r#"{"prompt_id": "p-1", "prompt_integration": "langfuse"}"#
        ))
    );
}

#[test]
fn the_update_trace_keys_flag_gates_the_requested_keys() {
    let mut inputs = Inputs::default();
    inputs.set_metadata("update_trace_keys", value(r#"["trace_metadata"]"#));
    assert!(cleaned(&inputs).steering.update_trace_keys.is_empty());
    inputs.update_trace_keys = true;
    assert_eq!(
        cleaned(&inputs).steering.update_trace_keys,
        vec!["trace_metadata".to_owned()]
    );
}

#[test]
fn the_masking_function_metadata_key_is_dropped() {
    let mut inputs = Inputs::default();
    inputs.set_metadata(
        "langfuse_masking_function",
        Json::Text("ignored".to_owned()),
    );
    assert!(
        !cleaned(&inputs)
            .fields
            .contains_key("langfuse_masking_function")
    );
}

#[test]
fn masking_applies_to_the_generation_input_and_output() {
    let mut case = fixture();
    case.inputs.masker = Some(upper);
    case.inputs.input = Some(value(r#"{"messages": ["hi"]}"#));
    case.inputs.output = Some(Json::Text("bye".to_owned()));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        json(generation, "langfuse.observation.input"),
        Some(value(r#"{"messages": ["HI"]}"#))
    );
    assert_eq!(
        text(generation, "langfuse.observation.output").as_deref(),
        Some("BYE")
    );
}

#[test]
fn mask_flags_replace_the_values_with_the_redaction_text() {
    let mut case = fixture();
    case.inputs.input = Some(Json::Text("secret input".to_owned()));
    case.inputs.output = Some(Json::Text("secret output".to_owned()));
    case.inputs.set_metadata("mask_input", Json::Bool(true));
    case.inputs
        .set_metadata("mask_output", Json::Text("true".to_owned()));
    case.inputs.set_metadata(
        "parent_observation_id",
        Json::Text("0000000000000005".to_owned()),
    );
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        text(generation, "langfuse.observation.input").as_deref(),
        Some("redacted-by-litellm")
    );
    assert_eq!(
        text(generation, "langfuse.observation.output").as_deref(),
        Some("redacted-by-litellm")
    );
    assert_eq!(
        text(generation, "langfuse.trace.input").as_deref(),
        Some("redacted-by-litellm")
    );
}

#[test]
fn redaction_drops_user_api_key_metadata_but_keeps_the_end_user() {
    let mut case = fixture();
    case.inputs.redact = true;
    case.inputs.payload = Some(payload(
        r#"{"user_api_key_alias": "prod-key", "user_api_key_end_user_id": "end-user-1"}"#,
    ));
    case.inputs
        .set_metadata("user_api_key_alias", Json::Text("prod-key".to_owned()));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert!(absent(
        generation,
        "langfuse.observation.metadata.user_api_key_alias"
    ));
    assert!(absent(
        generation,
        "langfuse.observation.metadata.user_api_key_end_user_id"
    ));
    assert_eq!(text(generation, "user.id").as_deref(), Some("end-user-1"));
    assert_eq!(generation.name.to_string(), "litellm-completion");
}

#[test]
fn default_tags_turn_metadata_keys_into_tags() {
    let mut case = fixture();
    case.inputs.payload = Some(payload("{}"));
    case.inputs.default_tags = vec!["user_api_key_alias".to_owned()];
    case.inputs
        .set_metadata("user_api_key_alias", Json::Text("prod-key".to_owned()));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        attribute(generation, "langfuse.trace.tags"),
        Some(string_array(&["team-a", "user_api_key_alias:prod-key"]))
    );
}

#[rstest]
#[case::hit(Some(true), "cache_hit:True")]
#[case::miss(Some(false), "cache_hit:False")]
#[case::unknown(None, "cache_hit:None")]
fn the_cache_hit_tag_reports_the_flag(#[case] cache_hit: Option<bool>, #[case] expected: &str) {
    let mut case = fixture();
    case.inputs.default_tags = vec!["cache_hit".to_owned()];
    case.inputs.cache_hit = cache_hit;
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        attribute(generation, "langfuse.trace.tags"),
        Some(string_array(&[expected]))
    );
}

#[rstest]
#[case::hidden_params(Some("hidden-9"), None, "cache_key:hidden-9")]
#[case::preset_key(None, Some("preset-1"), "cache_key:preset-1")]
#[case::unset(None, None, "cache_key:None")]
fn the_cache_key_tag_falls_back_to_the_preset_key(
    #[case] hidden_key: Option<&str>,
    #[case] preset: Option<&str>,
    #[case] expected: &str,
) {
    let mut case = fixture();
    case.inputs.default_tags = vec!["cache_key".to_owned()];
    case.inputs.preset_cache_key = preset.map(str::to_owned);
    if let Some(hidden) = hidden_key {
        case.inputs.set_metadata(
            "hidden_params",
            value(&format!(r#"{{"cache_key": "{hidden}"}}"#)),
        );
    }
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        attribute(generation, "langfuse.trace.tags"),
        Some(string_array(&[expected]))
    );
}

#[rstest]
#[case::configured(
    Some("https://proxy.example"),
    Some("proxy_base_url:https://proxy.example")
)]
#[case::unset(None, None)]
#[case::empty(Some(""), None)]
fn the_proxy_base_url_tag_reads_the_environment(
    #[case] base_url: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut case = fixture();
    case.inputs.default_tags = vec!["proxy_base_url".to_owned()];
    if let Some(base) = base_url {
        case.inputs
            .environment
            .insert("PROXY_BASE_URL".to_owned(), base.to_owned());
    }
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    match expected {
        Some(tag) => assert_eq!(
            attribute(generation, "langfuse.trace.tags"),
            Some(string_array(&[tag]))
        ),
        None => assert!(absent(generation, "langfuse.trace.tags")),
    }
}

#[test]
fn the_proxy_base_url_tag_needs_the_default_tag() {
    let mut case = fixture();
    case.inputs.environment.insert(
        "PROXY_BASE_URL".to_owned(),
        "https://proxy.example".to_owned(),
    );
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert!(absent(generation, "langfuse.trace.tags"));
}

#[test]
fn proxy_request_headers_never_reach_an_attribute() {
    let mut case = fixture();
    case.inputs.litellm_params = value(
        r#"{"proxy_server_request": {"method": "POST", "url": "http://proxy/v1/chat",
            "headers": {"Authorization": "Bearer secret", "Cookie": "a=b",
                        "Referer": "http://elsewhere", "X-Api-Key": "sk-leak"}}}"#,
    );
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert!(absent(
        generation,
        "langfuse.trace.metadata.proxy_server_request"
    ));
    let rendered = format!("{:?}", generation.attributes);
    assert!(!rendered.contains("Bearer secret"));
    assert!(!rendered.contains("sk-leak"));
    assert!(!rendered.contains("a=b"));
}

#[test]
fn debug_metadata_keeps_only_scalar_values() {
    let mut case = fixture();
    case.inputs.set_metadata("debug_langfuse", Json::Bool(true));
    case.inputs.set_metadata("text", Json::Text("t".to_owned()));
    case.inputs.set_metadata("flag", Json::Bool(true));
    case.inputs.set_metadata("count", value("5"));
    case.inputs.set_metadata("nested", value(r#"{"a": 1}"#));
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        json(
            generation,
            "langfuse.trace.metadata.metadata_passed_to_litellm"
        ),
        Some(value(
            r#"{"debug_langfuse": true, "text": "t", "flag": true, "count": 5}"#
        ))
    );
}

#[rstest]
#[case::true_flag(Json::Bool(true), true)]
#[case::false_flag(Json::Bool(false), false)]
#[case::upper_text(Json::Text("TRUE".to_owned()), true)]
#[case::other_text(Json::Text("yes".to_owned()), false)]
fn debug_metadata_responds_to_the_debug_flag(#[case] debug: Json, #[case] expected: bool) {
    let mut case = fixture();
    case.inputs.set_metadata("debug_langfuse", debug);
    let outcome = run(case.assemble());
    let generation = outcome.generation().expect("a generation span");
    assert_eq!(
        !absent(
            generation,
            "langfuse.trace.metadata.metadata_passed_to_litellm"
        ),
        expected
    );
}

#[test]
fn the_same_metadata_builds_the_settings_without_touching_the_caller() {
    let mut inputs = Inputs::default();
    inputs.set_metadata("session_id", Json::Text("session-x".to_owned()));
    let settings = AssemblySettings {
        default_tags: &[],
        update_trace_keys: false,
        redact_user_api_key_info: false,
    };
    let clean = metadata::clean(&AssemblyInput {
        trace: &inputs.trace(),
        settings: &settings,
    });
    assert_eq!(
        clean.steering.session_id,
        Some(Json::Text("session-x".to_owned()))
    );
    assert!(inputs.metadata.contains_key("session_id"));
}
