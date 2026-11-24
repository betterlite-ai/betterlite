#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_models::ocr_pricing::OcrUsage;
use betterlite_models::usage_sum::{Usage, UsageNumber};
use rust_decimal::Decimal;

use super::usage::{UsageNumberView, UsageWire};
use betterlite_core::cost::realtime::Event;

use super::{
    CollectUsageRequest, CombineUsageRequest, ImageRequest, OcrRequest, OcrUsageWire,
    RealtimeCostWire, RealtimeTranscriptionWire, RerankRequest, TranscriptionRequest,
    VectorStoreRequest, VideoRequest,
};
use crate::cost::convert::{Amount, ConvertError, encode, parse};

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("the expected decimal parses")
}

#[test]
fn cost_extra_request_ocr_usage_leaves_an_absent_annotation_at_zero() {
    let wire: OcrUsageWire =
        parse(r#"{"credits": null, "pages_processed": 3}"#).expect("a partial usage parses");
    let usage = OcrUsage::from(&wire);
    assert_eq!(usage.pages_processed, Some(3));
    assert_eq!(usage.pages_processed_annotation, 0);
    assert_eq!(usage.credits, None);

    let credits: OcrUsageWire =
        parse(r#"{"credits": 2.5, "pages_processed": null, "pages_processed_annotation": 4}"#)
            .expect("the credit usage parses");
    assert_eq!(OcrUsage::from(&credits).credits, Some(dec("2.5")));
}

#[test]
fn cost_extra_request_rejects_an_unknown_field() {
    let failure = parse::<OcrRequest>(
        r#"{"model": "m", "custom_llm_provider": null, "usage": null, "model_info": null, "x": 1}"#,
    );
    assert!(matches!(failure, Err(ConvertError::Json { .. })));
}

#[test]
fn cost_extra_request_malformed_text_is_an_error() {
    let failure = parse::<CombineUsageRequest>("{");
    assert!(matches!(failure, Err(ConvertError::Json { .. })));
    let wrong = parse::<CombineUsageRequest>(r#"{"usages": 3}"#);
    assert!(matches!(wrong, Err(ConvertError::Json { .. })));
}

#[test]
fn cost_extra_request_ocr_shapes_parse() {
    let ocr: OcrRequest = parse(
        r#"{"model": "mistral/mistral-ocr-latest", "custom_llm_provider": "mistral",
            "usage": {"credits": null, "pages_processed": 2, "pages_processed_annotation": null},
            "model_info": {"ocr_cost_per_page": 0.001}}"#,
    )
    .expect("ocr parses");
    assert_eq!(ocr.model, "mistral/mistral-ocr-latest");

    let batch: OcrRequest = parse(
        r#"{"model": "m", "custom_llm_provider": null, "usage": {"pages_processed": 1},
            "model_info": null}"#,
    )
    .expect("the batch ocr parses");
    assert_eq!(
        batch
            .usage
            .expect("the batch usage parses")
            .pages_processed_annotation,
        None
    );
}

#[test]
fn cost_extra_request_search_shapes_parse() {
    let vector: VectorStoreRequest = parse(
        r#"{"model": "vertex_ai/search_api", "custom_llm_provider": "vertex_ai",
            "response": {"object": "vector_store.search_results.page", "search_query": "q",
                         "data": []}}"#,
    )
    .expect("the vector store search parses");
    assert_eq!(vector.model.as_deref(), Some("vertex_ai/search_api"));

    let rerank: RerankRequest = parse(
        r#"{"model": "cohere/rerank", "custom_llm_provider": "cohere",
            "billed_units": {"search_units": 1}}"#,
    )
    .expect("the rerank parses");
    assert_eq!(
        rerank
            .billed_units
            .and_then(|units| units.search_units)
            .flatten(),
        Some(1)
    );
}

#[test]
fn cost_extra_request_media_shapes_parse() {
    let transcription: TranscriptionRequest =
        parse(r#"{"model": "whisper-1", "custom_llm_provider": null, "duration": 1.5}"#)
            .expect("the transcription parses");
    assert_eq!(transcription.duration.0, dec("1.5"));

    let image: ImageRequest = parse(
        r#"{"model": "dall-e-3", "custom_llm_provider": null, "quality": "standard", "n": 1,
            "size": "1024-x-1024", "model_info": null}"#,
    )
    .expect("the image parses");
    assert_eq!(image.n_images, Some(1));

    let video: VideoRequest = parse(
        r#"{"model": "veo-3.0-generate-001", "duration_seconds": 8,
            "custom_llm_provider": "vertex_ai", "model_info": null, "video_resolution": "720p"}"#,
    )
    .expect("the video parses");
    assert_eq!(video.video_resolution.as_deref(), Some("720p"));
}

#[test]
fn cost_extra_request_realtime_shapes_parse() {
    let realtime: RealtimeCostWire = parse(
        r#"{"results": [], "combined_usage": {"prompt_tokens": 1, "completion_tokens": 2,
            "total_tokens": 3, "prompt_tokens_details": null,
            "completion_tokens_details": null},
            "custom_llm_provider": "openai", "litellm_model_name": "gpt-realtime",
            "data_residency": null, "custom_pricing_model": null, "base_pricing_model": null}"#,
    )
    .expect("the realtime request parses");
    assert_eq!(realtime.litellm_model_name, "gpt-realtime");

    let realtime_transcription: RealtimeTranscriptionWire = parse(
        r#"{"results": [{"type": "session.created", "session": {"model": "gpt-4o-transcribe"}}],
            "custom_llm_provider": "openai", "litellm_model_name": "gpt-realtime",
            "custom_pricing_model": null}"#,
    )
    .expect("the realtime transcription request parses");
    assert_eq!(realtime_transcription.results.len(), 1);

    let collect: CollectUsageRequest =
        parse(r#"{"source": "responses_ws", "results": []}"#).expect("collect parses");
    assert_eq!(collect.results.len(), 0);
}

#[test]
fn cost_extra_request_usage_keeps_details_and_numeric_extras() {
    let wire: UsageWire = parse(
        r#"{"prompt_tokens": 10, "completion_tokens": 4, "total_tokens": 14,
            "prompt_tokens_details": {"text_tokens": 6, "cached_tokens": 2,
                                      "cache_write_tokens": 1, "cache_creation_tokens": 1},
            "completion_tokens_details": {"reasoning_tokens": 3},
            "requests": 2}"#,
    )
    .expect("the usage parses");
    let usage = Usage::from(&wire);
    let details = usage.prompt_tokens_details.expect("the details parse");
    assert_eq!(details.text_tokens, Some(6));
    assert_eq!(details.cached_tokens, Some(2));
    assert_eq!(details.cache_write_tokens, Some(1));
    assert_eq!(
        usage
            .completion_tokens_details
            .and_then(|completion| completion.reasoning_tokens),
        Some(3)
    );
    assert_eq!(
        usage.numeric_extras.get("requests"),
        Some(&UsageNumber::Integer(2))
    );
}

#[test]
fn cost_extra_result_pair_keeps_both_slots_as_numbers() {
    let encoded =
        encode(&[Amount(dec("0.00025")), Amount(Decimal::ZERO)]).expect("the pair encodes");
    let decoded: Vec<Amount> = parse(&encoded).expect("the pair parses back");
    assert_eq!(decoded[0].0, dec("0.00025"));
    assert_eq!(decoded[1].0, Decimal::ZERO);
    assert_eq!(encoded, "[0.00025,0]");
}

#[test]
fn cost_extra_result_cost_answer_holds_one_number() {
    let encoded = encode(&Amount(dec("1.5"))).expect("the answer encodes");
    assert_eq!(encoded, "1.5");
}

#[test]
fn cost_extra_result_usage_view_spells_the_extras_flat() {
    let mut extras = BTreeMap::new();
    extras.insert("requests".to_owned(), UsageNumber::Integer(2));
    extras.insert("byok".to_owned(), UsageNumber::Boolean(true));
    let usage = Usage {
        prompt_tokens: 3,
        completion_tokens: 1,
        total_tokens: 4,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        numeric_extras: extras,
    };
    let encoded = encode(&UsageWire::from(&usage)).expect("the usage answer encodes");
    assert_eq!(
        encoded,
        r#"{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4,"prompt_tokens_details":null,"completion_tokens_details":null,"byok":true,"requests":2}"#
    );
    let kind = UsageNumberView::from(&UsageNumber::Decimal(dec("2.5")));
    assert!(matches!(kind, UsageNumberView::Amount(amount) if amount.0 == dec("2.5")));
}

#[test]
fn cost_extra_result_partition_keeps_the_tier_and_its_events() {
    let tiers: Vec<(Option<String>, Vec<Event>)> = vec![(
        None,
        vec![parse(r#"{"type": "response.completed"}"#).expect("the event parses")],
    )];
    let encoded = encode(&tiers).expect("the partition answer encodes");
    assert_eq!(encoded, r#"[[null,[{"type":"response.completed"}]]]"#);
}
