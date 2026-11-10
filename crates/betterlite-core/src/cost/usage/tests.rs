#![cfg(test)]

use std::borrow::Cow;
use std::str::FromStr as _;

use betterlite_logging::fields::usage::Usage;
use betterlite_models::cost_usage::transcription::TranscriptionUsage;
use betterlite_models::decisions::Json;
use betterlite_models::decisions::openai::DecisionUsage;
use betterlite_models::decisions::systemone::DecisionsUsage;
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::routes::CallType;

use super::provider_cost::{hidden_response_cost, without_provider_stated_cost};
use super::tiers::{normalize_service_tier, resolve_billable_service_tier, served_service_tier};
use super::{
    BillingResponse, BillingUsage, BillingUsageError, ResponseKind, get_usage_object,
    infer_call_type, is_known_usage_object, transcription_has_tokens,
};

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn usage(prompt: i64, completion: i64) -> Usage {
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: prompt.checked_add(completion).expect("fits"),
        prompt_tokens_details: None,
        completion_tokens_details: None,
        server_tool_use: None,
        cost: None,
        extras: IndexMap::new(),
    }
}

fn mapping(text: &str) -> IndexMap<String, Json> {
    let parsed: Json = serde_json::from_str(text).expect("valid JSON");
    let Json::Object(fields) = parsed else {
        panic!("a mapping case must be an object");
    };
    fields
}

fn chat_response(usage: &Usage) -> BillingResponse<'_> {
    BillingResponse {
        kind: ResponseKind::Chat,
        usage: BillingUsage::Chat(usage),
        service_tier: None,
        hidden_params: None,
    }
}

fn details(usage: &Usage, name: &str) -> Option<i64> {
    let found = match name {
        "prompt_tokens_details" => usage.prompt_tokens_details.as_ref(),
        _ => usage.completion_tokens_details.as_ref(),
    }?;
    match found
        .get("cached_tokens")
        .or_else(|| found.get("text_tokens"))
    {
        Some(Json::Number(count)) => count.as_i64(),
        _ => None,
    }
}

#[test]
fn chat_usage_returns_the_existing_usage() {
    let existing = usage(5, 2);
    let response = chat_response(&existing);

    assert_eq!(
        get_usage_object(&response).expect("known usage"),
        Some(existing)
    );
}

#[test]
fn anthropic_mapping_wins_over_responses_detection() {
    let anthropic = mapping(
        r#"{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":3,"cache_creation_input_tokens":2}"#,
    );
    let response = BillingResponse {
        kind: ResponseKind::Chat,
        usage: BillingUsage::Mapping(&anthropic),
        service_tier: None,
        hidden_params: None,
    };
    let converted = get_usage_object(&response)
        .expect("converts")
        .expect("has usage");

    assert_eq!(converted.prompt_tokens, 15);
    assert_eq!(converted.completion_tokens, 5);
    assert_eq!(converted.total_tokens, 20);
    assert_eq!(details(&converted, "prompt_tokens_details"), Some(3));
    assert!(matches!(
        converted.prompt_tokens_details.as_ref().and_then(|found| found.get("cache_write_tokens")),
        Some(Json::Number(count)) if count.as_i64() == Some(2)
    ));
}

#[test]
fn responses_mapping_is_transformed_after_anthropic_detection_fails() {
    let responses = mapping(
        r#"{"input_tokens":4,"output_tokens":6,"input_tokens_details":{"cached_tokens":2,"text_tokens":2},"output_tokens_details":{"reasoning_tokens":1}}"#,
    );
    let response = BillingResponse {
        kind: ResponseKind::Chat,
        usage: BillingUsage::Mapping(&responses),
        service_tier: None,
        hidden_params: None,
    };
    let converted = get_usage_object(&response)
        .expect("converts")
        .expect("has usage");

    assert_eq!(converted.prompt_tokens, 4);
    assert_eq!(converted.completion_tokens, 6);
    assert_eq!(converted.total_tokens, 10);
    assert_eq!(
        converted
            .completion_tokens_details
            .as_ref()
            .and_then(|found| found.get("reasoning_tokens")),
        Some(&number(1))
    );
}

#[test]
fn interactions_mapping_is_transformed_before_the_generic_constructor() {
    let interactions = mapping(
        r#"{"total_input_tokens":10,"total_output_tokens":4,"total_cached_tokens":3,"input_tokens_by_modality":[{"modality":"text","tokens":10}]}"#,
    );
    let response = BillingResponse {
        kind: ResponseKind::Chat,
        usage: BillingUsage::Mapping(&interactions),
        service_tier: None,
        hidden_params: None,
    };
    let converted = get_usage_object(&response)
        .expect("converts")
        .expect("has usage");

    assert_eq!(converted.prompt_tokens, 10);
    assert_eq!(converted.completion_tokens, 4);
    assert_eq!(converted.total_tokens, 14);
    assert_eq!(
        converted.extras.get("cache_read_input_tokens"),
        Some(&number(3))
    );
}

#[test]
fn typed_transcription_duration_is_not_token_usage() {
    let duration: TranscriptionUsage =
        serde_json::from_str(r#"{"type":"duration","seconds":12.5}"#).expect("parses");
    let response = BillingResponse {
        kind: ResponseKind::Transcription,
        usage: BillingUsage::Transcription(&duration),
        service_tier: None,
        hidden_params: None,
    };
    assert_eq!(get_usage_object(&response).expect("converts"), None);

    let tokens: TranscriptionUsage = serde_json::from_str(
        r#"{"type":"tokens","input_tokens":7,"output_tokens":3,"total_tokens":10,"input_token_details":{"audio_tokens":5,"text_tokens":2}}"#,
    )
    .expect("parses");
    let token_response = BillingResponse {
        kind: ResponseKind::Transcription,
        usage: BillingUsage::Transcription(&tokens),
        service_tier: None,
        hidden_params: None,
    };
    let converted = get_usage_object(&token_response)
        .expect("converts")
        .expect("has usage");

    assert_eq!(converted.prompt_tokens, 7);
    assert_eq!(converted.completion_tokens, 3);
    assert_eq!(details(&converted, "prompt_tokens_details"), Some(2));
}

#[test]
fn decisions_usage_recomputes_the_total_and_keeps_cache_fields() {
    let ir: DecisionsUsage = serde_json::from_str(
        r#"{"input_tokens":9,"output_tokens":4,"cached_tokens":2,"cache_write_tokens":1}"#,
    )
    .expect("parses");
    let response = BillingResponse {
        kind: ResponseKind::DecisionsIr,
        usage: BillingUsage::DecisionsIr(&ir),
        service_tier: None,
        hidden_params: None,
    };
    let converted = get_usage_object(&response)
        .expect("converts")
        .expect("has usage");

    assert_eq!(converted.prompt_tokens, 9);
    assert_eq!(converted.completion_tokens, 4);
    assert_eq!(converted.total_tokens, 13);
    assert_eq!(details(&converted, "prompt_tokens_details"), Some(2));
    assert!(converted.completion_tokens_details.is_none());
}

#[test]
fn openai_decisions_usage_ignores_its_reported_total_and_reasoning() {
    let openai: DecisionUsage = serde_json::from_str(
        r#"{"input_tokens":9,"output_tokens":4,"total_tokens":99,"input_tokens_details":{"cached_tokens":2,"cache_write_tokens":1},"output_tokens_details":{"reasoning_tokens":3}}"#,
    )
    .expect("parses");
    let response = BillingResponse {
        kind: ResponseKind::DecisionsOpenAi,
        usage: BillingUsage::DecisionsOpenAi(&openai),
        service_tier: None,
        hidden_params: None,
    };
    let converted = get_usage_object(&response)
        .expect("converts")
        .expect("has usage");

    assert_eq!(converted.total_tokens, 13);
    assert!(converted.completion_tokens_details.is_none());
}

#[test]
fn decisions_response_without_usage_is_absent() {
    let response = BillingResponse {
        kind: ResponseKind::DecisionsIr,
        usage: BillingUsage::Absent,
        service_tier: None,
        hidden_params: None,
    };

    assert_eq!(get_usage_object(&response).expect("converts"), None);
}

#[test]
fn decisions_response_rejects_a_non_decision_usage() {
    let existing = usage(1, 1);
    let response = BillingResponse {
        kind: ResponseKind::DecisionsIr,
        usage: BillingUsage::Chat(&existing),
        service_tier: None,
        hidden_params: None,
    };

    assert!(matches!(
        get_usage_object(&response),
        Err(BillingUsageError::Projection { kind: "decision" })
    ));
}

#[test]
fn absent_and_null_usage_return_none() {
    let absent = BillingResponse {
        kind: ResponseKind::Chat,
        usage: BillingUsage::Absent,
        service_tier: None,
        hidden_params: None,
    };
    let null = BillingResponse {
        kind: ResponseKind::Chat,
        usage: BillingUsage::Null,
        service_tier: None,
        hidden_params: None,
    };

    assert_eq!(get_usage_object(&absent).expect("converts"), None);
    assert_eq!(get_usage_object(&null).expect("converts"), None);
}

#[test]
fn typed_records_use_the_generic_constructor_without_provider_detection() {
    let record = mapping(r#"{"input_tokens":4,"output_tokens":6,"total_tokens":10}"#);
    let response = BillingResponse {
        kind: ResponseKind::Chat,
        usage: BillingUsage::TypedRecord(&record),
        service_tier: None,
        hidden_params: None,
    };
    let converted = get_usage_object(&response)
        .expect("converts")
        .expect("has usage");

    assert_eq!(converted.prompt_tokens, 0);
    assert_eq!(converted.total_tokens, 10);
    assert_eq!(converted.extras.get("input_tokens"), Some(&number(4)));
}

#[test]
fn typed_responses_usage_uses_the_responses_converter() {
    let typed: ResponseApiUsage = serde_json::from_str(
        r#"{"input_tokens":4,"output_tokens":6,"total_tokens":10,"input_tokens_details":{"cached_tokens":2,"text_tokens":2},"output_tokens_details":{"reasoning_tokens":1},"service_tier":"flex"}"#,
    )
    .expect("parses");
    let response = BillingResponse {
        kind: ResponseKind::Responses,
        usage: BillingUsage::Response(&typed),
        service_tier: None,
        hidden_params: None,
    };
    let converted = get_usage_object(&response)
        .expect("converts")
        .expect("has usage");

    assert_eq!(converted.prompt_tokens, 4);
    assert_eq!(converted.completion_tokens, 6);
    assert_eq!(
        converted.extras.get("service_tier"),
        Some(&Json::Text("flex".to_owned()))
    );
}

#[test]
fn known_usage_recognizes_only_the_source_types() {
    let existing = usage(1, 1);
    let responses: ResponseApiUsage =
        serde_json::from_str(r#"{"input_tokens":1,"output_tokens":1,"total_tokens":2}"#)
            .expect("parses");
    let duration: TranscriptionUsage =
        serde_json::from_str(r#"{"type":"duration","seconds":1.0}"#).expect("parses");
    let ir: DecisionsUsage = serde_json::from_str("{}").expect("parses");
    let openai: DecisionUsage =
        serde_json::from_str(r#"{"input_tokens":1,"output_tokens":1,"total_tokens":2}"#)
            .expect("parses");
    let empty = IndexMap::new();

    assert!(is_known_usage_object(&BillingUsage::Chat(&existing)));
    assert!(is_known_usage_object(&BillingUsage::Response(&responses)));
    assert!(is_known_usage_object(&BillingUsage::Transcription(
        &duration
    )));
    assert!(is_known_usage_object(&BillingUsage::DecisionsIr(&ir)));
    assert!(is_known_usage_object(&BillingUsage::DecisionsOpenAi(
        &openai
    )));
    assert!(!is_known_usage_object(&BillingUsage::Mapping(&empty)));
    assert!(!is_known_usage_object(&BillingUsage::TypedRecord(&empty)));
    assert!(!is_known_usage_object(&BillingUsage::Interactions(&empty)));
    assert!(!is_known_usage_object(&BillingUsage::Absent));
}

#[rstest]
#[case::chat(ResponseKind::Chat, Some(CallType::Completion))]
#[case::chat_stream(ResponseKind::ChatStream, Some(CallType::Completion))]
#[case::embedding(ResponseKind::Embedding, Some(CallType::Embedding))]
#[case::transcription(ResponseKind::Transcription, Some(CallType::Transcription))]
#[case::speech(ResponseKind::Speech, Some(CallType::Speech))]
#[case::rerank(ResponseKind::Rerank, Some(CallType::Rerank))]
#[case::image(ResponseKind::Image, Some(CallType::ImageGeneration))]
#[case::text_completion(ResponseKind::TextCompletion, Some(CallType::TextCompletion))]
#[case::send_message(ResponseKind::SendMessage, Some(CallType::SendMessage))]
#[case::responses(ResponseKind::Responses, None)]
#[case::decisions(ResponseKind::DecisionsIr, None)]
#[case::other(ResponseKind::Other, None)]
fn infer_call_type_maps_tagged_response_kinds(
    #[case] kind: ResponseKind,
    #[case] expected: Option<CallType>,
) {
    assert_eq!(infer_call_type(None, kind), expected);
}

#[test]
fn infer_call_type_keeps_an_explicit_call_type() {
    assert_eq!(
        infer_call_type(Some(CallType::Embedding), ResponseKind::Chat),
        Some(CallType::Embedding)
    );
}

#[rstest]
#[case::no_details(None::<(i64, i64)>, 0, 0, false)]
#[case::prompt_tokens(None::<(i64, i64)>, 1, 0, true)]
#[case::completion_tokens(None::<(i64, i64)>, 0, 2, true)]
#[case::prompt_audio(Some((0, 4)), 0, 0, true)]
#[case::prompt_text(Some((3, 0)), 0, 0, true)]
#[case::zero_details(Some((0, 0)), 0, 0, false)]
fn transcription_token_details_accept_prompt_completion_or_audio(
    #[case] details: Option<(i64, i64)>,
    #[case] prompt: i64,
    #[case] completion: i64,
    #[case] expected: bool,
) {
    let mut usage = usage(prompt, completion);
    if let Some((text, audio)) = details {
        let mut record = IndexMap::new();
        record.insert("text_tokens".to_owned(), number(text));
        record.insert("audio_tokens".to_owned(), number(audio));
        usage.prompt_tokens_details = Some(record);
    }

    assert_eq!(
        transcription_has_tokens(Some(&usage)).expect("counts are integers"),
        expected
    );
}

#[test]
fn transcription_token_details_are_false_without_usage() {
    assert!(!transcription_has_tokens(None).expect("an absent usage holds no tokens"),);
}

#[rstest]
#[case::text("text_tokens", Json::Text("12".to_owned()))]
#[case::boolean("audio_tokens", Json::Bool(true))]
#[case::fraction("text_tokens", Json::Number(Number::from_f64(1.5).expect("finite")))]
#[case::large("audio_tokens", Json::Number(Number::from(u64::MAX)))]
fn transcription_token_details_reject_non_integer_counts(
    #[case] field: &'static str,
    #[case] value: Json,
) {
    let mut usage = usage(0, 0);
    let mut record = IndexMap::new();
    record.insert(field.to_owned(), value);
    usage.prompt_tokens_details = Some(record);

    assert!(matches!(
        transcription_has_tokens(Some(&usage)),
        Err(BillingUsageError::Token { field: counted }) if counted == field
    ));
}

#[test]
fn transcription_token_details_treat_null_as_zero() {
    let mut usage = usage(0, 0);
    let mut record = IndexMap::new();
    record.insert("audio_tokens".to_owned(), Json::Null);
    usage.prompt_tokens_details = Some(record);

    assert!(!transcription_has_tokens(Some(&usage)).expect("null counts as zero"),);
}

#[test]
fn without_provider_stated_cost_borrows_until_a_cost_exists() {
    let plain = usage(1, 2);
    assert!(matches!(
        without_provider_stated_cost(&plain),
        Cow::Borrowed(_)
    ));

    let mut priced = usage(1, 2);
    priced.cost = Some(Decimal::from_str("0.5").expect("decimal"));
    priced
        .extras
        .insert("service_tier".to_owned(), Json::Text("flex".to_owned()));
    let cleared = without_provider_stated_cost(&priced);
    let Cow::Owned(owned) = cleared else {
        panic!("a present cost must be removed from a copy");
    };

    assert_eq!(owned.cost, None);
    assert_eq!(owned.prompt_tokens, 1);
    assert_eq!(
        owned.extras.get("service_tier"),
        Some(&Json::Text("flex".to_owned()))
    );
}

#[test]
fn hidden_response_cost_is_absent_without_the_header() {
    let hidden = mapping(r#"{"additional_headers":{}}"#);
    assert_eq!(hidden_response_cost(Some(&hidden)).expect("converts"), None);
    assert_eq!(hidden_response_cost(None).expect("converts"), None);
}

#[rstest]
#[case::null_cost(r#"{"additional_headers":{"llm_provider-x-litellm-response-cost":null}}"#, None::<&str>)]
#[case::string_cost(
    r#"{"additional_headers":{"llm_provider-x-litellm-response-cost":"0.25"}}"#,
    Some("0.25")
)]
#[case::numeric_cost(
    r#"{"additional_headers":{"llm_provider-x-litellm-response-cost":0.5}}"#,
    Some("0.5")
)]
fn hidden_response_cost_reads_strings_and_numbers(
    #[case] text: &str,
    #[case] expected: Option<&str>,
) {
    let hidden = mapping(text);
    let found = hidden_response_cost(Some(&hidden)).expect("converts");

    assert_eq!(
        found,
        expected.map(|value| Decimal::from_str(value).expect("decimal"))
    );
}

#[rstest]
#[case::malformed_string(
    r#"{"additional_headers":{"llm_provider-x-litellm-response-cost":"abc"}}"#
)]
#[case::nan_string(r#"{"additional_headers":{"llm_provider-x-litellm-response-cost":"NaN"}}"#)]
fn hidden_response_cost_keeps_the_decimal_parse_error(#[case] text: &str) {
    let hidden = mapping(text);

    assert!(matches!(
        hidden_response_cost(Some(&hidden)),
        Err(BillingUsageError::HiddenNumber {
            field: "llm_provider-x-litellm-response-cost",
            ..
        })
    ));
}

#[rstest]
#[case::boolean(r#"{"additional_headers":{"llm_provider-x-litellm-response-cost":true}}"#)]
#[case::array(r#"{"additional_headers":{"llm_provider-x-litellm-response-cost":[]}}"#)]
#[case::non_object_headers(r#"{"additional_headers":"nope"}"#)]
fn hidden_response_cost_rejects_non_numeric_values(#[case] text: &str) {
    let hidden = mapping(text);

    assert!(matches!(
        hidden_response_cost(Some(&hidden)),
        Err(BillingUsageError::Hidden { .. })
    ));
}

#[test]
fn served_service_tier_returns_a_response_tier_verbatim() {
    let response = BillingResponse {
        kind: ResponseKind::Chat,
        usage: BillingUsage::Absent,
        service_tier: Some("PRIORITY"),
        hidden_params: None,
    };

    assert_eq!(
        served_service_tier(response.service_tier, response.hidden_params, None),
        Some("PRIORITY".to_owned())
    );
}

#[test]
fn served_service_tier_stops_at_an_empty_response_tier() {
    let mut usage = usage(1, 1);
    usage
        .extras
        .insert("service_tier".to_owned(), Json::Text("flex".to_owned()));
    let response = BillingResponse {
        kind: ResponseKind::Chat,
        usage: BillingUsage::Absent,
        service_tier: Some(""),
        hidden_params: None,
    };

    assert_eq!(
        served_service_tier(response.service_tier, response.hidden_params, Some(&usage)),
        Some(String::new())
    );
}

#[test]
fn served_service_tier_reads_the_usage_tier() {
    let mut usage = usage(1, 1);
    usage
        .extras
        .insert("service_tier".to_owned(), Json::Text("flex".to_owned()));
    let response = BillingResponse {
        kind: ResponseKind::Chat,
        usage: BillingUsage::Absent,
        service_tier: None,
        hidden_params: None,
    };

    assert_eq!(
        served_service_tier(response.service_tier, response.hidden_params, Some(&usage)),
        Some("flex".to_owned())
    );
}

#[rstest]
#[case::priority("ON_DEMAND_PRIORITY", Some("priority"))]
#[case::priority_lowercase("on_demand_priority", Some("priority"))]
#[case::flex("FLEX", Some("flex"))]
#[case::batch("BATCH", Some("flex"))]
#[case::on_demand_flex("ON_DEMAND_FLEX", Some("flex"))]
#[case::on_demand("ON_DEMAND", Some("default"))]
#[case::unknown_type("weird", Some("default"))]
#[case::empty_type("", None::<&str>)]
#[case::space_padded(" flex ", Some("default"))]
fn served_service_tier_maps_hidden_traffic_types(
    #[case] traffic_type: &str,
    #[case] expected: Option<&str>,
) {
    let hidden = IndexMap::from([(
        "provider_specific_fields".to_owned(),
        Json::Object(IndexMap::from([(
            "traffic_type".to_owned(),
            Json::Text(traffic_type.to_owned()),
        )])),
    )]);
    let response = BillingResponse {
        kind: ResponseKind::Chat,
        usage: BillingUsage::Absent,
        service_tier: None,
        hidden_params: Some(&hidden),
    };

    assert_eq!(
        served_service_tier(response.service_tier, response.hidden_params, None),
        expected.map(str::to_owned)
    );
}

#[test]
fn served_service_tier_is_absent_without_hidden_traffic() {
    let hidden = mapping(r#"{"provider_specific_fields":{}}"#);
    let response = BillingResponse {
        kind: ResponseKind::Chat,
        usage: BillingUsage::Absent,
        service_tier: None,
        hidden_params: Some(&hidden),
    };

    assert_eq!(
        served_service_tier(response.service_tier, response.hidden_params, None),
        None
    );
}

#[rstest]
#[case::auto(Some("auto"), None::<&str>)]
#[case::upper_auto(Some("AUTO"), None::<&str>)]
#[case::unknown(Some("Weird"), Some("Weird"))]
#[case::absent(None::<&str>, None::<&str>)]
fn normalize_service_tier_treats_auto_as_absent(
    #[case] tier: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(normalize_service_tier(tier), expected);
}

#[rstest]
#[case::served_priced(Some("priority"), Some("PRIORITY"), Some("priority"))]
#[case::served_fast(Some("priority"), Some("FAST"), Some("fast"))]
#[case::served_default(Some("priority"), Some("default"), None::<&str>)]
#[case::served_standard(Some("priority"), Some("Standard"), None::<&str>)]
#[case::served_unknown(Some("PRIORITY"), Some("weird"), Some("PRIORITY"))]
#[case::served_auto(Some("priority"), Some("auto"), Some("priority"))]
#[case::requested_auto(Some("auto"), None::<&str>, None::<&str>)]
#[case::nothing(None::<&str>, None::<&str>, None::<&str>)]
#[case::unknown_requested(Some("Weird"), None::<&str>, Some("Weird"))]
fn resolve_billable_service_tier_obeys_priced_and_base_tiers(
    #[case] requested: Option<&str>,
    #[case] served: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        resolve_billable_service_tier(requested, served),
        expected.map(str::to_owned)
    );
}
