#![cfg(test)]

use std::convert::Infallible;
use std::str::FromStr as _;

use async_trait::async_trait;
use betterlite_logging::fields::usage::Usage;
use betterlite_models::cost_usage::transcription::TranscriptionUsage;
use betterlite_models::decisions::Json;
use betterlite_models::decisions::systemone::DecisionsUsage;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::resolution::ProviderIdentity;
use betterlite_models::usage_sum::Usage as SumUsage;
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use betterlite_tokenizer::count::error::CountError;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::error::PrepareError;
use super::{
    AttemptBilling, AttemptOutcome, BillingTokenCounter, CompletionBillingInput,
    DictionaryResponse, PreparedBilling, PreparedBillingAttempt, PreparedCompletion, ResponseView,
    TypedResponse, prepare,
};
use crate::cost::context::ProviderInference;
use crate::cost::realtime::RealtimeBilling;
use crate::cost::usage::{BillingUsage, ResponseKind};
use crate::routes::CallType;

struct Lookup;

impl PriceLookup for Lookup {
    fn effective_model_info(&self, _model: &str, _provider: Option<&str>) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(&self, _model: &str, _provider: Option<&str>) -> Option<PricedModel> {
        None
    }
}

struct Inference;

impl ProviderInference for Inference {
    type Failure = Infallible;

    fn inferred_provider(&self, _model: &str) -> Result<ProviderIdentity, Self::Failure> {
        Ok(ProviderIdentity::Custom("openai".to_owned()))
    }
}

struct Counter;

#[async_trait]
impl BillingTokenCounter for Counter {
    async fn count_messages(&self, _model: &str, _messages: &[Json]) -> Result<usize, CountError> {
        Ok(3)
    }

    async fn count_text(&self, _model: &str, text: &str) -> Result<usize, CountError> {
        Ok(text.chars().count())
    }
}

fn json(document: &str) -> Json {
    serde_json::from_str(document).expect("the fixture is JSON")
}

fn map(document: &str) -> IndexMap<String, Json> {
    let Json::Object(found) = json(document) else {
        panic!("the fixture is an object");
    };
    found
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: &str) -> Json {
    Json::Number(Number::from_str(value).expect("the fixture is a number"))
}

fn fields(entries: &[(&str, Json)]) -> IndexMap<String, Json> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn chat_usage(prompt: i64, completion: i64, details: Option<&str>) -> Usage {
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: prompt.checked_add(completion).expect("the fixture fits"),
        prompt_tokens_details: details.map(map),
        completion_tokens_details: None,
        server_tool_use: None,
        cost: None,
        extras: IndexMap::new(),
    }
}

fn chat_view(usage: &Usage) -> ResponseView<'_> {
    ResponseView::Typed(TypedResponse {
        kind: ResponseKind::Chat,
        model: None,
        usage: BillingUsage::Chat(usage),
        service_tier: None,
        hidden_params: None,
    })
}

fn mapping_view(kind: ResponseKind, record: &IndexMap<String, Json>) -> ResponseView<'_> {
    ResponseView::Typed(TypedResponse {
        kind,
        model: None,
        usage: BillingUsage::Mapping(record),
        service_tier: None,
        hidden_params: None,
    })
}

fn base_input<'request>(
    response: Option<&'request ResponseView<'request>>,
) -> CompletionBillingInput<'request> {
    CompletionBillingInput {
        model: None,
        prompt: "",
        messages: &[],
        completion: "",
        call_type: None,
        custom_llm_provider: Some("openai"),
        region_name: None,
        router_model_id: None,
        custom_pricing: None,
        base_model: None,
        optional_params: None,
        service_tier: None,
        response,
    }
}

async fn prepare_attempts(input: &CompletionBillingInput<'_>) -> PreparedBilling {
    let completed = prepare(input, &Lookup, &Inference, &Counter)
        .await
        .expect("the input prepares");
    match completed {
        PreparedCompletion::Attempts(found) => *found,
        PreparedCompletion::WebsocketPartitions(_) => panic!("expected attempts"),
    }
}

fn counts(attempt: &PreparedBillingAttempt) -> (i64, i64, i64, i64) {
    match &attempt.outcome {
        AttemptOutcome::Ready(found) => (
            found.prompt_tokens,
            found.completion_tokens,
            found.cache_creation_input_tokens,
            found.cache_read_input_tokens,
        ),
        AttemptOutcome::Failed(error) => panic!("the attempt failed: {error}"),
    }
}

fn attempt_billing(attempt: &PreparedBillingAttempt) -> &AttemptBilling {
    match &attempt.outcome {
        AttemptOutcome::Ready(found) => found,
        AttemptOutcome::Failed(error) => panic!("the attempt failed: {error}"),
    }
}

#[tokio::test]
async fn candidates_keep_the_selected_response_and_request_models() {
    let usage = chat_usage(10, 5, None);
    let view = ResponseView::Typed(TypedResponse {
        kind: ResponseKind::Chat,
        model: Some("gpt-4o"),
        usage: BillingUsage::Chat(&usage),
        service_tier: None,
        hidden_params: None,
    });
    let mut input = base_input(Some(&view));
    input.model = Some("gpt-4o");
    let prepared = prepare_attempts(&input).await;

    assert_eq!(prepared.selected_model.as_deref(), Some("openai/gpt-4o"));
    let models: Vec<Option<&str>> = prepared
        .attempts
        .iter()
        .map(|attempt| attempt.model.as_deref())
        .collect();
    assert_eq!(
        models,
        [Some("openai/gpt-4o"), Some("gpt-4o"), Some("gpt-4o")]
    );
}

#[tokio::test]
async fn null_candidates_stay_as_attempts_that_require_a_model() {
    let input = base_input(None);
    let prepared = prepare_attempts(&input).await;

    assert_eq!(prepared.attempts.len(), 2);
    assert!(prepared.attempts.iter().all(|attempt| {
        matches!(
            &attempt.outcome,
            AttemptOutcome::Failed(PrepareError::ModelMissing)
        )
    }));
}

#[rstest]
#[case::image(CallType::ImageGeneration, "", "azure", "dall-e-2")]
#[case::async_image(CallType::AimageGeneration, "", "azure", "dall-e-2")]
#[case::completion(CallType::Completion, "", "azure", "")]
#[case::other_provider(CallType::ImageGeneration, "", "openai", "")]
#[case::named_model(CallType::ImageGeneration, "dall-e-3", "azure", "dall-e-3")]
#[tokio::test]
async fn azure_empty_image_models_become_dall_e_2(
    #[case] call_type: CallType,
    #[case] model: &str,
    #[case] provider: &str,
    #[case] expected: &str,
) {
    let mut input = base_input(None);
    input.model = Some(model);
    input.custom_llm_provider = Some(provider);
    input.call_type = Some(call_type);
    let prepared = prepare_attempts(&input).await;

    assert_eq!(
        prepared
            .attempts
            .last()
            .and_then(|attempt| attempt.model.as_deref()),
        Some(expected)
    );
}

#[rstest]
#[case::raw_counters(
    r#"{"prompt_tokens":10,"completion_tokens":5,"cache_creation_input_tokens":2,"cache_read_input_tokens":3}"#,
    (10, 5, 2, 3)
)]
#[case::empty_details_keep_the_cache_key(
    r#"{"prompt_tokens":10,"prompt_tokens_details":{},"cache_read_input_tokens":3}"#,
    (10, 0, 0, 3)
)]
#[case::details_without_cached_zero_the_cache_read(
    r#"{"prompt_tokens":10,"prompt_tokens_details":{"text_tokens":4},"cache_read_input_tokens":3}"#,
    (10, 0, 0, 0)
)]
#[case::null_counters_read_as_zero(
    r#"{"prompt_tokens":null,"completion_tokens":null,"cache_read_input_tokens":null}"#,
    (0, 0, 0, 0)
)]
#[case::anthropic_without_output_tokens(
    r#"{"input_tokens":10,"cache_read_input_tokens":3}"#,
    (13, 0, 0, 3)
)]
#[case::output_only_cache_stays_raw(
    r#"{"output_tokens":6,"cache_read_input_tokens":3}"#,
    (0, 0, 0, 3)
)]
#[case::prompt_with_interactions_keys_stays_raw(
    r#"{"prompt_tokens":5,"total_input_tokens":10,"cache_read_input_tokens":4}"#,
    (5, 0, 0, 4)
)]
#[tokio::test]
async fn mapping_usage_counts_like_the_source(
    #[case] document: &str,
    #[case] expected: (i64, i64, i64, i64),
) {
    let record = map(document);
    let view = mapping_view(ResponseKind::Chat, &record);
    let input = base_input(Some(&view));
    let prepared = prepare_attempts(&input).await;

    assert_eq!(counts(&prepared.attempts[0]), expected);
}

#[tokio::test]
async fn anthropic_usage_is_converted_before_counting() {
    let record = map(
        r#"{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":3,"cache_creation_input_tokens":2}"#,
    );
    let view = mapping_view(ResponseKind::Chat, &record);
    let input = base_input(Some(&view));
    let prepared = prepare_attempts(&input).await;

    assert_eq!(counts(&prepared.attempts[0]), (15, 5, 2, 3));
}

#[tokio::test]
async fn responses_usage_is_converted_before_counting() {
    let record = map(
        r#"{"input_tokens":4,"output_tokens":6,"input_tokens_details":{"cached_tokens":2,"text_tokens":2}}"#,
    );
    let view = mapping_view(ResponseKind::Chat, &record);
    let input = base_input(Some(&view));
    let prepared = prepare_attempts(&input).await;

    assert_eq!(counts(&prepared.attempts[0]), (4, 6, 0, 2));
}

#[rstest]
#[case::full_record(
    r#"{"total_input_tokens":10,"total_output_tokens":5,"total_cached_tokens":2}"#,
    (10, 5, 0, 2)
)]
#[case::input_only(r#"{"total_input_tokens":10}"#, (10, 0, 0, 0))]
#[tokio::test]
async fn interactions_usage_is_converted_before_counting(
    #[case] document: &str,
    #[case] expected: (i64, i64, i64, i64),
) {
    let record = map(document);
    let view = mapping_view(ResponseKind::Chat, &record);
    let input = base_input(Some(&view));
    let prepared = prepare_attempts(&input).await;

    assert_eq!(counts(&prepared.attempts[0]), expected);
}

#[tokio::test]
async fn decisions_usage_counts_its_prompt_details() {
    let usage: DecisionsUsage = serde_json::from_str(
        r#"{"input_tokens":7,"output_tokens":3,"cached_tokens":2,"cache_write_tokens":1}"#,
    )
    .expect("the decisions usage is readable");
    let view = ResponseView::Typed(TypedResponse {
        kind: ResponseKind::DecisionsIr,
        model: None,
        usage: BillingUsage::DecisionsIr(&usage),
        service_tier: None,
        hidden_params: None,
    });
    let input = base_input(Some(&view));
    let prepared = prepare_attempts(&input).await;

    assert_eq!(counts(&prepared.attempts[0]), (7, 3, 0, 2));
}

#[tokio::test]
async fn typed_chat_details_override_the_cache_read() {
    let usage = chat_usage(10, 5, Some(r#"{"cached_tokens":4}"#));
    let view = chat_view(&usage);
    let input = base_input(Some(&view));
    let prepared = prepare_attempts(&input).await;

    assert_eq!(counts(&prepared.attempts[0]), (10, 5, 0, 4));
}

#[tokio::test]
async fn typed_chat_usage_reads_the_cache_creation_extra() {
    let mut usage = chat_usage(10, 5, None);
    usage.extras.insert(
        "cache_creation_input_tokens".to_owned(),
        Json::Number(Number::from(2_i64)),
    );
    let view = chat_view(&usage);
    let input = base_input(Some(&view));
    let prepared = prepare_attempts(&input).await;

    assert_eq!(counts(&prepared.attempts[0]), (10, 5, 2, 0));
}

#[tokio::test]
async fn a_malformed_cache_creation_extra_fails_the_attempt() {
    let mut usage = chat_usage(10, 5, None);
    usage.extras.insert(
        "cache_creation_input_tokens".to_owned(),
        Json::Text("two".to_owned()),
    );
    let view = chat_view(&usage);
    let input = base_input(Some(&view));
    let prepared = prepare_attempts(&input).await;

    assert!(matches!(
        &prepared.attempts[0].outcome,
        AttemptOutcome::Failed(PrepareError::Count {
            field: "cache_creation_input_tokens"
        })
    ));
}

#[tokio::test]
async fn typed_responses_usage_counts_its_details() {
    let usage: ResponseApiUsage = serde_json::from_str(
        r#"{"input_tokens":4,"output_tokens":6,"total_tokens":10,"input_tokens_details":{"cached_tokens":2}}"#,
    )
    .expect("the responses usage is readable");
    let view = ResponseView::Typed(TypedResponse {
        kind: ResponseKind::Chat,
        model: None,
        usage: BillingUsage::Response(&usage),
        service_tier: None,
        hidden_params: None,
    });
    let input = base_input(Some(&view));
    let prepared = prepare_attempts(&input).await;

    assert_eq!(counts(&prepared.attempts[0]), (4, 6, 0, 2));
}

#[rstest]
#[case::tokens(
    r#"{"type":"tokens","input_tokens":7,"output_tokens":3,"total_tokens":10,"input_token_details":{"audio_tokens":5,"text_tokens":2}}"#,
    (7, 3, 0, 0)
)]
#[case::duration(r#"{"type":"duration","seconds":12.5}"#, (0, 0, 0, 0))]
#[tokio::test]
async fn typed_transcription_usage_counts_tokens_only(
    #[case] document: &str,
    #[case] expected: (i64, i64, i64, i64),
) {
    let usage: TranscriptionUsage =
        serde_json::from_str(document).expect("the transcription usage is readable");
    let view = ResponseView::Typed(TypedResponse {
        kind: ResponseKind::Transcription,
        model: None,
        usage: BillingUsage::Transcription(&usage),
        service_tier: None,
        hidden_params: None,
    });
    let input = base_input(Some(&view));
    let prepared = prepare_attempts(&input).await;

    assert_eq!(counts(&prepared.attempts[0]), expected);
}

#[tokio::test]
async fn absent_usage_counts_nothing() {
    let view = ResponseView::Typed(TypedResponse {
        kind: ResponseKind::Chat,
        model: None,
        usage: BillingUsage::Null,
        service_tier: None,
        hidden_params: None,
    });
    let input = base_input(Some(&view));
    let prepared = prepare_attempts(&input).await;

    assert_eq!(counts(&prepared.attempts[0]), (0, 0, 0, 0));
}

#[tokio::test]
async fn an_unknown_typed_usage_is_replaced_for_later_attempts() {
    let record =
        map(r#"{"prompt_tokens":10,"completion_tokens":5,"cache_creation_input_tokens":7}"#);
    let view = ResponseView::Typed(TypedResponse {
        kind: ResponseKind::Chat,
        model: None,
        usage: BillingUsage::TypedRecord(&record),
        service_tier: None,
        hidden_params: None,
    });
    let input = base_input(Some(&view));
    let prepared = prepare_attempts(&input).await;

    assert_eq!(counts(&prepared.attempts[0]), (10, 5, 7, 0));
    assert_eq!(counts(&prepared.attempts[1]), (10, 5, 7, 0));
    let replaced = prepared
        .replaced_usage
        .as_ref()
        .expect("the usage was replaced");
    assert_eq!(replaced.prompt_tokens, 10);
    assert_eq!(replaced.completion_tokens, 5);
}

#[tokio::test]
async fn token_counting_prefers_messages_then_prompt() {
    let messages = vec![json(r#"{"role":"user","content":"not counted"}"#)];
    let mut input = base_input(None);
    input.model = Some("gpt-4o");
    input.messages = &messages;
    input.prompt = "ignored";
    input.completion = "abcdef";
    let prepared = prepare_attempts(&input).await;

    assert_eq!(counts(&prepared.attempts[0]), (3, 6, 0, 0));
}

#[tokio::test]
async fn token_counting_uses_the_prompt_when_messages_are_empty() {
    let mut input = base_input(None);
    input.model = Some("gpt-4o");
    input.prompt = "abcd";
    input.completion = "xy";
    let prepared = prepare_attempts(&input).await;

    assert_eq!(counts(&prepared.attempts[0]), (4, 2, 0, 0));
}

#[rstest]
#[case::explicit_wins(Some("flex"), Some("priority"), Some("standard"), Some("flex"))]
#[case::auto_falls_to_the_requested(Some("auto"), Some("priority"), None, Some("priority"))]
#[case::served_priced_tier_wins(None, Some("flex"), Some("PRIORITY"), Some("priority"))]
#[case::served_standard_cancels_the_requested(None, Some("flex"), Some("standard"), None)]
#[case::served_alone_decides(None, None, Some("flex"), Some("flex"))]
#[case::unknown_served_keeps_the_requested(None, Some("flex"), Some("unknown"), Some("flex"))]
#[tokio::test]
async fn resolves_the_billable_tier(
    #[case] explicit: Option<&str>,
    #[case] requested: Option<&str>,
    #[case] served: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let usage = chat_usage(10, 5, None);
    let view = ResponseView::Typed(TypedResponse {
        kind: ResponseKind::Chat,
        model: None,
        usage: BillingUsage::Chat(&usage),
        service_tier: served,
        hidden_params: None,
    });
    let params = fields(&[("service_tier", requested.map_or(Json::Null, text))]);
    let mut input = base_input(Some(&view));
    input.service_tier = explicit;
    input.optional_params = requested.map(|_tier| &params);
    let prepared = prepare_attempts(&input).await;

    assert_eq!(prepared.service_tier.as_deref(), expected);
}

#[tokio::test]
async fn hidden_params_choose_the_model_and_the_served_tier() {
    let usage = chat_usage(10, 5, None);
    let hidden = map(
        r#"{"provider_response_model":"gpt-4o-mini","provider_specific_fields":{"traffic_type":"ON_DEMAND_PRIORITY"}}"#,
    );
    let view = ResponseView::Typed(TypedResponse {
        kind: ResponseKind::Chat,
        model: Some("gpt-4o-2024"),
        usage: BillingUsage::Chat(&usage),
        service_tier: None,
        hidden_params: Some(&hidden),
    });
    let mut input = base_input(Some(&view));
    input.model = Some("gpt-4o");
    let prepared = prepare_attempts(&input).await;

    assert_eq!(
        prepared.selected_model.as_deref(),
        Some("openai/gpt-4o-mini")
    );
    assert_eq!(prepared.service_tier.as_deref(), Some("priority"));
}

#[tokio::test]
async fn hidden_attributes_rebind_provider_and_region() {
    let usage = chat_usage(10, 5, None);
    let hidden = map(r#"{"custom_llm_provider":"azure","region_name":"eastus"}"#);
    let view = ResponseView::Typed(TypedResponse {
        kind: ResponseKind::Chat,
        model: None,
        usage: BillingUsage::Chat(&usage),
        service_tier: None,
        hidden_params: Some(&hidden),
    });
    let input = base_input(Some(&view));
    let prepared = prepare_attempts(&input).await;

    assert!(
        prepared
            .attempts
            .iter()
            .all(|attempt| attempt.custom_llm_provider.as_deref() == Some("azure"))
    );
    assert!(
        prepared
            .attempts
            .iter()
            .all(|attempt| attempt.region_name.as_deref() == Some("eastus"))
    );
}

#[tokio::test]
async fn hidden_nulls_overwrite_provider_and_region() {
    let usage = chat_usage(10, 5, None);
    let hidden = map(r#"{"custom_llm_provider":null,"region_name":null}"#);
    let view = ResponseView::Typed(TypedResponse {
        kind: ResponseKind::Chat,
        model: None,
        usage: BillingUsage::Chat(&usage),
        service_tier: None,
        hidden_params: Some(&hidden),
    });
    let mut input = base_input(Some(&view));
    input.region_name = Some("us");
    let prepared = prepare_attempts(&input).await;

    assert_eq!(prepared.attempts[0].custom_llm_provider, None);
    assert_eq!(prepared.attempts[0].region_name, None);
}

#[tokio::test]
async fn an_empty_provider_is_dropped_for_typed_responses_only() {
    let usage = chat_usage(10, 5, None);
    let empty = map("{}");
    let typed = ResponseView::Typed(TypedResponse {
        kind: ResponseKind::Chat,
        model: None,
        usage: BillingUsage::Chat(&usage),
        service_tier: None,
        hidden_params: Some(&empty),
    });
    let mut input = base_input(Some(&typed));
    input.custom_llm_provider = Some("");
    let prepared = prepare_attempts(&input).await;

    assert_eq!(prepared.attempts[0].custom_llm_provider, None);
}

#[tokio::test]
async fn a_realtime_response_drops_its_empty_provider() {
    let record = RealtimeBilling {
        results: Vec::new(),
        usage: SumUsage::default(),
        service_tier: None,
    };
    let usage = chat_usage(1, 1, None);
    let view = ResponseView::Realtime {
        record: &record,
        usage: &usage,
    };
    let mut input = base_input(Some(&view));
    input.custom_llm_provider = Some("");
    let prepared = prepare_attempts(&input).await;

    assert_eq!(prepared.attempts[0].custom_llm_provider, None);
}

#[tokio::test]
async fn a_dictionary_response_keeps_its_empty_provider() {
    let record = map(r#"{"prompt_tokens":1,"_hidden_params":{"custom_llm_provider":"azure"}}"#);
    let dictionary = ResponseView::Dictionary(DictionaryResponse {
        model: None,
        usage: BillingUsage::Mapping(&record),
        service_tier: None,
    });
    let mut input = base_input(Some(&dictionary));
    input.custom_llm_provider = Some("");
    let prepared = prepare_attempts(&input).await;

    assert_eq!(
        prepared.attempts[0].custom_llm_provider.as_deref(),
        Some("")
    );
}

#[tokio::test]
async fn custom_pricing_clears_a_provider_stated_cost() {
    let mut usage = chat_usage(10, 5, None);
    usage.cost = Some(Decimal::new(3, 2));
    let view = chat_view(&usage);
    let mut input = base_input(Some(&view));
    input.custom_pricing = Some(true);
    let prepared = prepare_attempts(&input).await;

    let cleared = prepared.cost_usage.as_ref().expect("the usage is kept");
    assert_eq!(cleared.cost, None);
    assert!(prepared.explicit_pricing);
}

#[tokio::test]
async fn a_usage_cost_survives_without_truthy_custom_pricing() {
    let mut usage = chat_usage(10, 5, None);
    usage.cost = Some(Decimal::new(3, 2));
    let view = chat_view(&usage);
    let mut input = base_input(Some(&view));
    input.custom_pricing = Some(false);
    input.base_model = Some("azure/gpt-4");
    let prepared = prepare_attempts(&input).await;

    assert_eq!(
        prepared.cost_usage.as_ref().and_then(|found| found.cost),
        Some(Decimal::new(3, 2))
    );
    assert!(prepared.explicit_pricing);
}

fn billable_event(service_tier: &str, prompt: &str, completion: &str) -> IndexMap<String, Json> {
    fields(&[
        ("type", text("response.completed")),
        (
            "response",
            Json::Object(fields(&[
                (
                    "usage",
                    Json::Object(fields(&[
                        ("prompt_tokens", number(prompt)),
                        ("completion_tokens", number(completion)),
                    ])),
                ),
                ("service_tier", text(service_tier)),
            ])),
        ),
    ])
}

#[tokio::test]
async fn websocket_tiers_partition_into_recursive_records() {
    let record = RealtimeBilling {
        results: vec![
            billable_event("priority", "10", "2"),
            billable_event("flex", "4", "1"),
        ],
        usage: SumUsage::default(),
        service_tier: None,
    };
    let usage = Usage::default();
    let view = ResponseView::Realtime {
        record: &record,
        usage: &usage,
    };
    let mut input = base_input(Some(&view));
    input.call_type = Some(CallType::PrivateAresponsesWebsocket);
    let completed = prepare(&input, &Lookup, &Inference, &Counter)
        .await
        .expect("the session partitions");

    let PreparedCompletion::WebsocketPartitions(parts) = completed else {
        panic!("expected websocket partitions");
    };
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].service_tier.as_deref(), Some("priority"));
    assert_eq!(parts[1].service_tier.as_deref(), Some("flex"));
    assert_eq!(parts[0].usage.prompt_tokens, 10);
    assert_eq!(parts[1].usage.prompt_tokens, 4);
}

#[tokio::test]
async fn a_single_websocket_tier_prices_normally() {
    let record = RealtimeBilling {
        results: vec![billable_event("priority", "10", "2")],
        usage: SumUsage::default(),
        service_tier: None,
    };
    let usage = chat_usage(10, 2, None);
    let view = ResponseView::Realtime {
        record: &record,
        usage: &usage,
    };
    let mut input = base_input(Some(&view));
    input.call_type = Some(CallType::PrivateAresponsesWebsocket);
    let prepared = prepare_attempts(&input).await;

    assert_eq!(prepared.call_type, CallType::PrivateAresponsesWebsocket);
    assert_eq!(counts(&prepared.attempts[0]), (10, 2, 0, 0));
}

#[tokio::test]
async fn call_type_defaults_to_completion() {
    let mut input = base_input(None);
    input.model = Some("gpt-4o");
    let prepared = prepare_attempts(&input).await;

    assert_eq!(prepared.call_type, CallType::Completion);
    assert!(!prepared.explicit_pricing);
    assert_eq!(
        attempt_billing(&prepared.attempts[0]).transcription_duration,
        Decimal::ZERO
    );
}

#[tokio::test]
async fn a_realtime_response_without_the_websocket_call_type_does_not_partition() {
    let record = RealtimeBilling {
        results: vec![
            billable_event("priority", "10", "2"),
            billable_event("flex", "4", "1"),
        ],
        usage: SumUsage::default(),
        service_tier: None,
    };
    let usage = chat_usage(1, 1, None);
    let view = ResponseView::Realtime {
        record: &record,
        usage: &usage,
    };
    let input = base_input(Some(&view));
    let prepared = prepare_attempts(&input).await;

    assert_eq!(prepared.attempts.len(), 2);
    assert_eq!(counts(&prepared.attempts[0]), (1, 1, 0, 0));
}
