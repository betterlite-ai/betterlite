use std::str::FromStr as _;

use betterlite_config::sections::settings::JsonValue;
use betterlite_core::routes::CallType;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::super::lift::deployment::{
    PROXY_REJECTED_BEFORE_ROUTING, deployment_attribution, stamp_deployment_attribution,
};
use super::super::lift::{
    INPUT_ESTIMABLE_CALL_TYPES, NO_UPSTREAM_LLM_CALL_KEY, PASS_THROUGH_CALL_TYPE,
    call_type_for_route, call_type_name, count_request_input_tokens, failure_fields_to_lift,
    failure_usage_to_lift, prompt_block_text, system_prompt_text,
};
use super::support::{
    FakeCatalog, FakeCounter, FakeRoutes, Journal, deployment, details, json, object,
};

#[tokio::test]
async fn recovered_usage_wins_over_the_estimate() {
    let journal = Journal::default();
    let counter = FakeCounter::new(&journal);
    let mut details = details();
    details.combined_usage_object = Some(json(
        r#"{"prompt_tokens":7,"completion_tokens":2,"total_tokens":9}"#,
    ));
    details.response_cost = Some(Decimal::new(25, 2));
    let (usage, cost) = failure_usage_to_lift(&details, &object(&[]), true, &counter)
        .await
        .expect("recovered usage");
    assert_eq!(
        usage,
        json(r#"{"prompt_tokens":7,"completion_tokens":2,"total_tokens":9}"#)
    );
    assert_eq!(cost, Decimal::new(25, 2));
    assert!(journal.entries().is_empty());
}

#[tokio::test]
async fn recovered_usage_without_a_cost_lifts_zero() {
    let journal = Journal::default();
    let counter = FakeCounter::new(&journal);
    let mut details = details();
    details.combined_usage_object = Some(json(r#"{"prompt_tokens":7}"#));
    let (_, cost) = failure_usage_to_lift(&details, &object(&[]), false, &counter)
        .await
        .expect("recovered usage");
    assert_eq!(cost, Decimal::ZERO);
}

#[tokio::test]
async fn dispatched_failures_estimate_input_usage_from_messages() {
    let journal = Journal::default();
    let counter = FakeCounter::new(&journal);
    let mut details = details();
    details.call_type = Some("acompletion".to_owned());
    details.model = Some("gpt-4".to_owned());
    details.messages = Some(vec![json(r#"{"role":"user","content":"hi"}"#)]);
    let (usage, cost) = failure_usage_to_lift(&details, &object(&[]), true, &counter)
        .await
        .expect("estimated usage");
    assert_eq!(
        usage,
        json(r#"{"prompt_tokens":11,"completion_tokens":0,"total_tokens":11}"#)
    );
    assert_eq!(cost, Decimal::ZERO);
    assert_eq!(journal.entries(), vec!["count_messages:gpt-4:1".to_owned()]);
}

#[tokio::test]
async fn system_prompt_text_counts_before_the_messages() {
    let journal = Journal::default();
    let counter = FakeCounter::new(&journal);
    let mut details = details();
    details.call_type = Some("acompletion".to_owned());
    details.model = Some("gpt-4".to_owned());
    details.messages = Some(vec![json(r#"{"role":"user","content":"hi"}"#)]);
    details.optional_params = Some(object(&[(
        "system",
        json(r#"[{"text":"be nice"},{"text":" and brief"}]"#),
    )]));
    let (usage, _) = failure_usage_to_lift(&details, &object(&[]), true, &counter)
        .await
        .expect("estimated usage");
    assert_eq!(
        usage,
        json(r#"{"prompt_tokens":14,"completion_tokens":0,"total_tokens":14}"#)
    );
    assert_eq!(
        journal.entries(),
        vec![
            "count_text:gpt-4:be nice and brief".to_owned(),
            "count_messages:gpt-4:1".to_owned()
        ]
    );
}

#[tokio::test]
async fn the_request_body_backfills_the_system_prompt() {
    let journal = Journal::default();
    let counter = FakeCounter::new(&journal);
    let mut details = details();
    details.call_type = Some("acompletion".to_owned());
    details.model = Some("gpt-4".to_owned());
    details.messages = Some(vec![json(r#"{"role":"user","content":"hi"}"#)]);
    details.optional_params = Some(object(&[]));
    let body = object(&[("system", json("\"anthropic system\""))]);
    let (usage, _) = failure_usage_to_lift(&details, &body, true, &counter)
        .await
        .expect("estimated usage");
    assert_eq!(
        usage,
        json(r#"{"prompt_tokens":14,"completion_tokens":0,"total_tokens":14}"#)
    );
}

#[derive(Clone, Copy)]
enum EstimateCase {
    NotDispatched,
    NoUpstreamCall,
    MissingCallType,
    NotEstimable,
}

#[rstest]
#[case::not_dispatched(EstimateCase::NotDispatched)]
#[case::no_upstream_call(EstimateCase::NoUpstreamCall)]
#[case::missing_call_type(EstimateCase::MissingCallType)]
#[case::not_estimable(EstimateCase::NotEstimable)]
#[tokio::test]
async fn no_estimate_without_dispatch_an_upstream_call_or_an_estimable_type(
    #[case] case: EstimateCase,
) {
    let journal = Journal::default();
    let counter = FakeCounter::new(&journal);
    let mut details = details();
    details.model = Some("gpt-4".to_owned());
    details.messages = Some(vec![json(r#"{"role":"user","content":"hi"}"#)]);
    let dispatched = match case {
        EstimateCase::NotDispatched => false,
        EstimateCase::NoUpstreamCall => {
            details.no_upstream_llm_call = true;
            details.call_type = Some("acompletion".to_owned());
            true
        }
        EstimateCase::MissingCallType => true,
        EstimateCase::NotEstimable => {
            details.call_type = Some("call_mcp_tool".to_owned());
            true
        }
    };
    assert!(
        failure_usage_to_lift(&details, &object(&[]), dispatched, &counter)
            .await
            .is_none()
    );
    assert!(journal.entries().is_empty());
}

#[tokio::test]
async fn a_tokenizer_failure_or_a_zero_count_gives_no_estimate() {
    let journal = Journal::default();
    let mut details = details();
    details.call_type = Some("acompletion".to_owned());
    details.model = Some("gpt-4".to_owned());
    details.messages = Some(vec![json("\"hello\"")]);
    let failing = FakeCounter::failing(&journal);
    assert!(
        failure_usage_to_lift(&details, &object(&[]), true, &failing)
            .await
            .is_none()
    );
    let zero = FakeCounter::returning(&journal, 0);
    assert!(
        failure_usage_to_lift(&details, &object(&[]), true, &zero)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn text_entries_are_joined_into_one_count() {
    let journal = Journal::default();
    let counter = FakeCounter::new(&journal);
    let entries = vec![json("\"a\""), json("\"b\"")];
    let system = json("\"sys\"");
    let tokens = count_request_input_tokens("gpt-4", Some(&entries), Some(&system), &counter)
        .await
        .expect("counted");
    assert_eq!(tokens, 6);
    assert_eq!(
        journal.entries(),
        vec![
            "count_text:gpt-4:sys".to_owned(),
            "count_text:gpt-4:ab".to_owned()
        ]
    );
}

#[tokio::test]
async fn a_missing_or_empty_input_counts_only_the_system_prompt() {
    let journal = Journal::default();
    let counter = FakeCounter::new(&journal);
    let system = json("\"sys\"");
    assert_eq!(
        count_request_input_tokens("gpt-4", None, Some(&system), &counter)
            .await
            .expect("counted"),
        3
    );
    assert_eq!(
        count_request_input_tokens("gpt-4", Some(&[]), Some(&system), &counter)
            .await
            .expect("counted"),
        3
    );
}

#[test]
fn prompt_blocks_keep_strings_and_text_fields_only() {
    assert_eq!(prompt_block_text(&json("\"plain\"")), "plain");
    assert_eq!(prompt_block_text(&json(r#"{"text":"block"}"#)), "block");
    assert_eq!(prompt_block_text(&json(r#"{"type":"image"}"#)), "");
    assert_eq!(
        system_prompt_text(&json(r#"[{"text":"a"},"b",{"type":"image"}]"#)),
        "ab"
    );
}

#[test]
fn a_single_deployment_stamps_its_provider_model_info_and_model() {
    let journal = Journal::default();
    let mut single = deployment(Some("openai"), "gpt-4");
    single.model_info.insert("id".to_owned(), json("\"dep-1\""));
    let catalog = FakeCatalog::new(&journal).with("gpt-4", vec![single]);
    let attribution = deployment_attribution(Some("gpt-4"), Some("team"), &catalog);
    assert_eq!(attribution.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        attribution
            .model_info
            .as_ref()
            .and_then(|info| info.get("id")),
        Some(&json("\"dep-1\""))
    );
    assert_eq!(attribution.deployment.as_deref(), Some("gpt-4"));
    assert_eq!(
        journal.entries(),
        vec!["model_list:gpt-4:Some(\"team\")".to_owned()]
    );
}

#[test]
fn several_deployments_keep_a_shared_provider_only() {
    let journal = Journal::default();
    let same = FakeCatalog::new(&journal).with(
        "gpt-4",
        vec![
            deployment(Some("openai"), "gpt-4"),
            deployment(Some("openai"), "gpt-4-mini"),
        ],
    );
    let attribution = deployment_attribution(Some("gpt-4"), None, &same);
    assert_eq!(attribution.custom_llm_provider.as_deref(), Some("openai"));
    assert!(attribution.model_info.is_none());
    assert!(attribution.deployment.is_none());
    let mixed = FakeCatalog::new(&journal).with(
        "gpt-4",
        vec![
            deployment(Some("openai"), "gpt-4"),
            deployment(Some("azure"), "gpt-4"),
        ],
    );
    assert!(
        deployment_attribution(Some("gpt-4"), None, &mixed)
            .custom_llm_provider
            .is_none()
    );
}

#[test]
fn an_unknown_or_absent_group_has_no_attribution() {
    let journal = Journal::default();
    let catalog = FakeCatalog::new(&journal);
    let empty = deployment_attribution(Some("missing"), None, &catalog);
    assert_eq!(empty.custom_llm_provider, None);
    assert!(empty.model_info.is_none());
    let absent = deployment_attribution(None, None, &catalog);
    assert_eq!(absent.custom_llm_provider, None);
    assert_eq!(
        journal.entries(),
        vec!["model_list:missing:None".to_owned()]
    );
}

#[test]
fn stamping_a_proxy_rejection_marks_the_request_and_fills_metadata() {
    let journal = Journal::default();
    let mut single = deployment(Some("openai"), "gpt-4");
    single.model_info.insert("id".to_owned(), json("\"dep-1\""));
    let catalog = FakeCatalog::new(&journal).with("gpt-4", vec![single]);
    let mut params = object(&[("metadata", json(r#"{"existing":"yes"}"#))]);
    let attribution =
        stamp_deployment_attribution(&mut params, Some("gpt-4"), None, false, &catalog);
    assert_eq!(attribution.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        params.get(PROXY_REJECTED_BEFORE_ROUTING),
        Some(&json("true"))
    );
    assert_eq!(params.get("custom_llm_provider"), Some(&json("\"openai\"")));
    assert!(params.get("model_info").is_none());
    let metadata = params
        .get("metadata")
        .and_then(|value| value.as_object())
        .expect("metadata");
    assert_eq!(metadata.get("existing"), Some(&json("\"yes\"")));
    assert_eq!(metadata.get("deployment"), Some(&json("\"gpt-4\"")));
    assert_eq!(metadata.get("model_group"), Some(&json("\"gpt-4\"")));
    assert_eq!(
        metadata.get("model_info").and_then(|info| info.get("id")),
        Some(&json("\"dep-1\""))
    );
}

#[test]
fn a_dispatched_failure_keeps_the_router_metadata() {
    let journal = Journal::default();
    let catalog =
        FakeCatalog::new(&journal).with("gpt-4", vec![deployment(Some("openai"), "gpt-4")]);
    let mut params = object(&[("metadata", json(r#"{"model_info":{"id":"router"}}"#))]);
    stamp_deployment_attribution(&mut params, Some("gpt-4"), None, true, &catalog);
    assert_eq!(params.get("custom_llm_provider"), Some(&json("\"openai\"")));
    assert!(params.get(PROXY_REJECTED_BEFORE_ROUTING).is_none());
    let metadata = params
        .get("metadata")
        .and_then(|value| value.as_object())
        .expect("metadata");
    assert_eq!(metadata.get("deployment"), None);
}

#[test]
fn existing_metadata_wins_over_the_attribution() {
    let journal = Journal::default();
    let catalog =
        FakeCatalog::new(&journal).with("gpt-4", vec![deployment(Some("openai"), "gpt-4")]);
    let mut params = object(&[(
        "metadata",
        json(r#"{"model_info":{"id":"mine"},"deployment":"mine","model_group":"mine"}"#),
    )]);
    stamp_deployment_attribution(&mut params, Some("gpt-4"), None, false, &catalog);
    let metadata = params
        .get("metadata")
        .and_then(|value| value.as_object())
        .expect("metadata");
    assert_eq!(metadata.get("deployment"), Some(&json("\"mine\"")));
    assert_eq!(metadata.get("model_group"), Some(&json("\"mine\"")));
    assert_eq!(
        metadata.get("model_info").and_then(|info| info.get("id")),
        Some(&json("\"mine\""))
    );
}

#[test]
fn a_non_object_metadata_value_is_left_alone() {
    let journal = Journal::default();
    let catalog =
        FakeCatalog::new(&journal).with("gpt-4", vec![deployment(Some("openai"), "gpt-4")]);
    let mut params = object(&[("metadata", json("\"not an object\""))]);
    stamp_deployment_attribution(&mut params, Some("gpt-4"), None, false, &catalog);
    assert_eq!(params.get("metadata"), Some(&json("\"not an object\"")));
    assert_eq!(
        params.get(PROXY_REJECTED_BEFORE_ROUTING),
        Some(&json("true"))
    );
}

#[test]
fn a_group_without_one_deployment_does_not_create_metadata() {
    let journal = Journal::default();
    let catalog = FakeCatalog::new(&journal).with(
        "gpt-4",
        vec![
            deployment(Some("openai"), "gpt-4"),
            deployment(Some("openai"), "gpt-4-mini"),
        ],
    );
    let mut params = object(&[]);
    stamp_deployment_attribution(&mut params, Some("gpt-4"), None, false, &catalog);
    assert!(params.get("metadata").is_none());
    assert_eq!(
        params.get(PROXY_REJECTED_BEFORE_ROUTING),
        Some(&json("true"))
    );
}

#[test]
fn the_routes_single_operation_wins_and_mixed_routes_do_not() {
    let routes = FakeRoutes::default()
        .with(
            "/v1/embeddings",
            vec![CallType::Embedding, CallType::Aembedding],
        )
        .with("/mixed", vec![CallType::Acompletion, CallType::Embedding]);
    assert_eq!(
        call_type_for_route(Some("/v1/embeddings"), &routes),
        Some(CallType::Embedding)
    );
    assert_eq!(call_type_for_route(Some("/mixed"), &routes), None);
    assert_eq!(call_type_for_route(None, &routes), None);
    assert_eq!(call_type_for_route(Some("/unknown"), &routes), None);
}

#[rstest]
#[case::completion(CallType::Completion, "completion")]
#[case::acompletion(CallType::Acompletion, "acompletion")]
#[case::text_completion(CallType::TextCompletion, "text_completion")]
#[case::atext_completion(CallType::AtextCompletion, "atext_completion")]
#[case::anthropic_messages(CallType::AnthropicMessages, "anthropic_messages")]
#[case::aanthropic_messages(CallType::AanthropicMessages, "aanthropic_messages")]
#[case::responses(CallType::Responses, "responses")]
#[case::aresponses(CallType::Aresponses, "aresponses")]
#[case::embedding(CallType::Embedding, "embedding")]
#[case::aembedding(CallType::Aembedding, "aembedding")]
#[case::moderation(CallType::Moderation, "moderation")]
#[case::amoderation(CallType::Amoderation, "amoderation")]
#[case::image_generation(CallType::ImageGeneration, "image_generation")]
#[case::aimage_generation(CallType::AimageGeneration, "aimage_generation")]
#[case::speech(CallType::Speech, "speech")]
#[case::aspeech(CallType::Aspeech, "aspeech")]
#[case::rerank(CallType::Rerank, "rerank")]
#[case::arerank(CallType::Arerank, "arerank")]
#[case::generate_content(CallType::GenerateContent, "generate_content")]
#[case::agenerate_content(CallType::AgenerateContent, "agenerate_content")]
#[case::generate_stream(CallType::GenerateContentStream, "generate_content_stream")]
#[case::agenerate_stream(CallType::AgenerateContentStream, "agenerate_content_stream")]
fn estimable_call_types_are_spelled_like_litellm(#[case] call_type: CallType, #[case] name: &str) {
    assert!(INPUT_ESTIMABLE_CALL_TYPES.contains(&name));
    assert_eq!(
        serde_json::to_value(call_type).expect("serialized"),
        json(&format!("\"{name}\""))
    );
}

#[tokio::test]
async fn failure_fields_lift_only_the_present_values() {
    let journal = Journal::default();
    let counter = FakeCounter::new(&journal);
    let mut details = details();
    details.start_time = Some(json("\"2026-01-01T00:00:00+00:00\""));
    details.first_api_call_start_time = Some(json("\"2026-01-01T00:00:01+00:00\""));
    details.call_type = Some("acompletion".to_owned());
    details.standard_logging_object = Some(json(r#"{"model":"gpt-4"}"#));
    let fields = failure_fields_to_lift(&details, &object(&[]), &counter)
        .await
        .expect("fields");
    assert_eq!(
        fields.start_time,
        Some(json("\"2026-01-01T00:00:00+00:00\""))
    );
    assert_eq!(
        fields.first_api_call_start_time,
        Some(json("\"2026-01-01T00:00:01+00:00\""))
    );
    assert_eq!(fields.call_type, Some(json("\"acompletion\"")));
    assert_eq!(
        fields.standard_logging_object,
        Some(json(r#"{"model":"gpt-4"}"#))
    );
    assert_eq!(fields.combined_usage_object, None);
    assert_eq!(fields.response_cost, None);
    let mut data = object(&[]);
    fields.apply(&mut data);
    assert_eq!(
        data.get("start_time"),
        Some(&json("\"2026-01-01T00:00:00+00:00\""))
    );
    assert_eq!(
        data.get("first_api_call_start_time"),
        Some(&json("\"2026-01-01T00:00:01+00:00\""))
    );
    assert_eq!(data.get("call_type"), Some(&json("\"acompletion\"")));
    assert!(data.get("response_cost").is_none());
}

#[tokio::test]
async fn a_lifted_cost_is_a_json_number() {
    let journal = Journal::default();
    let counter = FakeCounter::new(&journal);
    let mut details = details();
    details.combined_usage_object = Some(json(r#"{"prompt_tokens":7}"#));
    details.response_cost = Some(Decimal::new(25, 2));
    let fields = failure_fields_to_lift(&details, &object(&[]), &counter)
        .await
        .expect("fields");
    assert_eq!(
        fields.response_cost,
        Some(JsonValue::Number(Number::from_str("0.25").expect("number")))
    );
}

#[test]
fn the_logging_keys_are_spelled_like_litellm() {
    assert_eq!(NO_UPSTREAM_LLM_CALL_KEY, "litellm_no_upstream_llm_call");
    assert_eq!(
        PROXY_REJECTED_BEFORE_ROUTING,
        "proxy_rejected_before_routing"
    );
    assert_eq!(PASS_THROUGH_CALL_TYPE, "pass_through_endpoint");
    assert_eq!(
        call_type_name(CallType::PassThroughEndpoint).as_deref(),
        Some(PASS_THROUGH_CALL_TYPE)
    );
}
