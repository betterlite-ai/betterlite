#![cfg(test)]

use std::str::FromStr as _;
use std::time::{Duration, SystemTime};

use betterlite_config::cost_adjustments::CostAdjustmentMap;
use betterlite_core::cost::realtime::Event;
use betterlite_models::decisions::Json;
use betterlite_models::ocr_pricing::OcrCostError;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::cost::convert::{Amount, parse};
use crate::cost::settings::{Settings, SettingsError, SettingsWire};
use crate::cost_extra::convert::{
    BatchRequest, CollectUsageRequest, CombineUsageRequest, ImageRequest, OcrRequest,
    PartitionUsageRequest, RealtimeCostWire, RealtimeTranscriptionWire, RerankRequest,
    TranscriptionRequest, VectorStoreRequest, VideoRequest,
};
use crate::cost_extra::engine::Engine;
use crate::cost_extra::engine::support::{collect_usage, combine_usage, partition_usage};
use crate::cost_extra::error::{RealtimeFailure, RerankFailure};

const CATALOG: &str = r#"{
    "ocr-test": {
        "litellm_provider": "mistral",
        "mode": "ocr",
        "ocr_cost_per_page": 0.001,
        "annotation_cost_per_page": 0.002,
        "ocr_cost_per_page_batches": 0.0005
    },
    "ocr-credit-test": {
        "litellm_provider": "mistral",
        "mode": "ocr",
        "ocr_cost_per_credit": 0.01,
        "ocr_cost_per_page": 0.5
    },
    "vertex_ai/search_api": {
        "litellm_provider": "vertex_ai",
        "mode": "vector_store",
        "input_cost_per_query": 0.0015
    },
    "rerank-test": {
        "litellm_provider": "cohere",
        "mode": "rerank",
        "input_cost_per_query": 0.002
    },
    "voyage-rerank": {
        "litellm_provider": "voyage",
        "mode": "rerank",
        "input_cost_per_query": 0.5,
        "input_cost_per_token": 0.00001
    },
    "jina-rerank": {
        "litellm_provider": "jina_ai",
        "mode": "rerank",
        "input_cost_per_token": 0.00002
    },
    "whisper-test": {
        "litellm_provider": "openai",
        "mode": "audio_transcription",
        "output_cost_per_second": 0.0001
    },
    "prompt-rate-test": {
        "litellm_provider": "openai",
        "mode": "audio_speech",
        "input_cost_per_second": 0.0002
    },
    "realtime-test": {
        "litellm_provider": "openai",
        "mode": "realtime",
        "input_cost_per_token": 0.00001,
        "output_cost_per_token": 0.00002
    },
    "transcribe-test": {
        "litellm_provider": "openai",
        "mode": "audio_transcription",
        "input_cost_per_second": 0.0001
    },
    "image-test": {
        "litellm_provider": "openai",
        "mode": "image_generation",
        "input_cost_per_image": 0.5
    },
    "video-test": {
        "litellm_provider": "vertex_ai",
        "mode": "video_generation",
        "output_cost_per_video_per_second": 0.4
    },
    "video-resolution-test": {
        "litellm_provider": "vertex_ai",
        "mode": "video_generation",
        "output_cost_per_second_720p": 0.1
    },
    "batch-test": {
        "litellm_provider": "openai",
        "mode": "chat",
        "input_cost_per_token": 0.0000025,
        "output_cost_per_token": 0.00001
    },
    "batch-rate-test": {
        "litellm_provider": "openai",
        "mode": "chat",
        "input_cost_per_token": 0.00001,
        "input_cost_per_token_batches": 0.000001,
        "output_cost_per_token": 0.00002,
        "output_cost_per_token_batches": 0.000002,
        "regional_processing_uplift_multiplier_eu": 1.1
    }
}"#;

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("the fixture decimal parses")
}

fn engine() -> Engine {
    let model_cost: IndexMap<String, Json> = parse(CATALOG).expect("the fixture catalog parses");
    let settings = Settings::build(SettingsWire {
        model_cost,
        cost_discount_config: CostAdjustmentMap::new(),
        cost_margin_config: CostAdjustmentMap::new(),
    })
    .expect("the fixture settings build");
    Engine { settings }
}

fn request<T>(text: &str) -> T
where
    T: for<'text> serde::Deserialize<'text>,
{
    parse(text).expect("the fixture request parses")
}

#[test]
fn cost_extra_engine_prices_ocr_pages_annotation_and_credits() {
    let pages: OcrRequest = request(
        r#"{"model": "ocr-test", "custom_llm_provider": "mistral",
            "usage": {"credits": null, "pages_processed": 5, "pages_processed_annotation": 2},
            "model_info": null}"#,
    );
    assert_eq!(
        engine().ocr_cost(&pages).expect("the pages are priced"),
        (Amount(dec("0.009")), Amount(Decimal::ZERO))
    );

    let credits: OcrRequest = request(
        r#"{"model": "ocr-credit-test", "custom_llm_provider": "mistral",
            "usage": {"credits": 3, "pages_processed": 5, "pages_processed_annotation": 0},
            "model_info": null}"#,
    );
    assert_eq!(
        engine().ocr_cost(&credits).expect("the credits are priced"),
        (Amount(dec("0.03")), Amount(Decimal::ZERO))
    );

    let missing: OcrRequest = request(
        r#"{"model": "ocr-test", "custom_llm_provider": "mistral",
            "usage": {"credits": null, "pages_processed": null, "pages_processed_annotation": 0},
            "model_info": null}"#,
    );
    assert!(matches!(
        engine().ocr_cost(&missing),
        Err(OcrCostError::MissingPages { .. })
    ));
}

#[test]
fn cost_extra_engine_bills_ocr_batches_at_the_batch_rate() {
    let batch: OcrRequest = request(
        r#"{"model": "ocr-test", "custom_llm_provider": "mistral",
            "usage": {"pages_processed": 4, "pages_processed_annotation": 2},
            "model_info": null}"#,
    );
    assert_eq!(
        engine()
            .ocr_batch_cost(&batch)
            .expect("the batch pages are priced"),
        (Amount(dec("0.006")), Amount(Decimal::ZERO))
    );

    let deployment: OcrRequest = request(
        r#"{"model": "unlisted-ocr", "custom_llm_provider": "mistral",
            "usage": {"pages_processed": 4, "pages_processed_annotation": 0},
            "model_info": {"ocr_cost_per_page_batches": 0.25}}"#,
    );
    assert_eq!(
        engine()
            .ocr_batch_cost(&deployment)
            .expect("the deployment pages are priced"),
        (Amount(dec("1")), Amount(Decimal::ZERO))
    );
}

#[test]
fn cost_extra_engine_prices_only_the_vertex_search_api() {
    let search: VectorStoreRequest = request(
        r#"{"model": "vertex_ai/search_api", "custom_llm_provider": "vertex_ai",
            "response": {"object": "vector_store.search_results.page", "search_query": "q", "data": []}}"#,
    );
    assert_eq!(
        engine()
            .vector_store_cost(&search)
            .expect("the search is priced"),
        (Amount(dec("0.0015")), Amount(Decimal::ZERO))
    );

    let rag: VectorStoreRequest = request(
        r#"{"model": "vertex_ai/rag_api", "custom_llm_provider": "vertex_ai", "response": {}}"#,
    );
    assert_eq!(
        engine()
            .vector_store_cost(&rag)
            .expect("the rag query is free"),
        (Amount(Decimal::ZERO), Amount(Decimal::ZERO))
    );

    let defaulted: VectorStoreRequest =
        request(r#"{"model": null, "custom_llm_provider": null, "response": {}}"#);
    assert_eq!(
        engine()
            .vector_store_cost(&defaulted)
            .expect("openai declares no vector price"),
        (Amount(Decimal::ZERO), Amount(Decimal::ZERO))
    );
}

#[test]
fn cost_extra_engine_rejects_unknown_vector_providers() {
    let unknown: VectorStoreRequest =
        request(r#"{"model": null, "custom_llm_provider": "typo-provider", "response": {}}"#);
    assert_eq!(
        engine()
            .vector_store_cost(&unknown)
            .expect_err("an unknown provider is refused")
            .to_string(),
        "'typo-provider' is not a valid LlmProviders"
    );

    let registry_only: VectorStoreRequest =
        request(r#"{"model": null, "custom_llm_provider": "sarvam", "response": {}}"#);
    assert_eq!(
        engine()
            .vector_store_cost(&registry_only)
            .expect_err("a registry-only provider is refused")
            .to_string(),
        "'sarvam' is not a valid LlmProviders"
    );
}

#[test]
fn cost_extra_engine_prices_rerank_units_by_provider() {
    let query: RerankRequest = request(
        r#"{"model": "rerank-test", "custom_llm_provider": "cohere",
            "billed_units": {"search_units": 2}}"#,
    );
    assert_eq!(
        engine()
            .rerank_cost(&query)
            .expect("the query units are priced"),
        (Amount(dec("0.004")), Amount(Decimal::ZERO))
    );

    let absent: RerankRequest = request(
        r#"{"model": "rerank-test", "custom_llm_provider": "cohere", "billed_units": null}"#,
    );
    assert_eq!(
        engine()
            .rerank_cost(&absent)
            .expect("no units price nothing"),
        (Amount(Decimal::ZERO), Amount(Decimal::ZERO))
    );

    let voyage: RerankRequest = request(
        r#"{"model": "voyage-rerank", "custom_llm_provider": "voyage",
            "billed_units": {"total_tokens": 100, "search_units": 9}}"#,
    );
    assert_eq!(
        engine()
            .rerank_cost(&voyage)
            .expect("voyage bills the tokens"),
        (Amount(dec("0.001")), Amount(Decimal::ZERO))
    );

    let jina: RerankRequest = request(
        r#"{"model": "jina-rerank", "custom_llm_provider": "jina_ai",
            "billed_units": {"total_tokens": 50}}"#,
    );
    assert_eq!(
        engine().rerank_cost(&jina).expect("jina bills the tokens"),
        (Amount(dec("0.001")), Amount(Decimal::ZERO))
    );
}

#[test]
fn cost_extra_engine_prices_a_transcription_duration() {
    let output: TranscriptionRequest =
        request(r#"{"model": "whisper-test", "custom_llm_provider": null, "duration": 2.5}"#);
    assert_eq!(
        engine()
            .transcription_cost(&output)
            .expect("the seconds are priced"),
        (Amount(Decimal::ZERO), Amount(dec("0.00025")))
    );

    let prompt: TranscriptionRequest = request(
        r#"{"model": "prompt-rate-test", "custom_llm_provider": "openai", "duration": 2.5}"#,
    );
    assert_eq!(
        engine()
            .transcription_cost(&prompt)
            .expect("the prompt seconds are priced"),
        (Amount(dec("0.0005")), Amount(Decimal::ZERO))
    );
}

#[test]
fn cost_extra_engine_prices_an_image_from_the_map_or_the_deployment() {
    let mapped: ImageRequest = request(
        r#"{"model": "image-test", "custom_llm_provider": null, "quality": null, "n": 2,
            "size": "1024-x-1024", "model_info": null}"#,
    );
    assert_eq!(
        engine().image_cost(&mapped).expect("the images are priced"),
        Amount(dec("1"))
    );

    let deployment: ImageRequest = request(
        r#"{"model": "unlisted-image", "custom_llm_provider": null, "quality": null, "n": 1,
            "size": "100x100", "model_info": {"input_cost_per_pixel": 0.000001}}"#,
    );
    assert_eq!(
        engine()
            .image_cost(&deployment)
            .expect("the deployment pixels are priced"),
        Amount(dec("0.01"))
    );
}

#[test]
fn cost_extra_engine_prices_a_video_duration() {
    let per_video: VideoRequest = request(
        r#"{"model": "video-test", "duration_seconds": 8, "custom_llm_provider": "vertex_ai",
            "model_info": null, "video_resolution": "720p"}"#,
    );
    assert_eq!(
        engine()
            .video_cost(&per_video)
            .expect("the per-video rate wins"),
        Amount(dec("3.2"))
    );

    let per_second: VideoRequest = request(
        r#"{"model": "video-resolution-test", "duration_seconds": 8,
            "custom_llm_provider": "vertex_ai", "model_info": null, "video_resolution": "720p"}"#,
    );
    assert_eq!(
        engine()
            .video_cost(&per_second)
            .expect("the resolution rate prices"),
        Amount(dec("0.8"))
    );
}

#[test]
fn cost_extra_engine_halves_the_batch_rate_without_batch_prices() {
    let request: BatchRequest = request(
        r#"{"usage": {"prompt_tokens": 1000, "completion_tokens": 500, "total_tokens": 1500,
            "prompt_tokens_details": null, "completion_tokens_details": null},
            "model": "batch-test", "custom_llm_provider": "openai", "model_info": null,
            "data_residency": null}"#,
    );
    assert_eq!(
        engine().batch_cost(&request).expect("the half rate prices"),
        (Amount(dec("0.00125")), Amount(dec("0.0025")))
    );
}

#[test]
fn cost_extra_engine_applies_the_batch_rates_and_the_regional_uplift() {
    let request: BatchRequest = request(
        r#"{"usage": {"prompt_tokens": 1000, "completion_tokens": 500, "total_tokens": 1500,
            "prompt_tokens_details": null, "completion_tokens_details": null},
            "model": "batch-rate-test", "custom_llm_provider": "openai", "model_info": null,
            "data_residency": "eu"}"#,
    );
    assert_eq!(
        engine()
            .batch_cost(&request)
            .expect("the batch rates price"),
        (Amount(dec("0.0011")), Amount(dec("0.0011")))
    );
}

const COMBINED_USAGE_REQUEST: &str = r#"{"usages": [
            {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15,
             "prompt_tokens_details": {"text_tokens": 6, "cached_tokens": 4,
                                       "cache_creation_tokens": 3, "audio_tokens": null,
                                       "image_tokens": null, "video_tokens": null,
                                       "web_search_requests": null,
                                       "google_maps_grounding_requests": null,
                                       "tool_use_tokens": null, "character_count": null,
                                       "image_count": null, "video_length_seconds": null,
                                       "audio_length_seconds": null, "query_count": null,
                                       "cache_write_tokens": 3, "cached_tokens_details": null},
             "completion_tokens_details": null, "requests": 2},
            {"prompt_tokens": 20, "completion_tokens": 7, "total_tokens": 27,
             "prompt_tokens_details": {"text_tokens": 19, "cached_tokens": 1,
                                       "cache_creation_tokens": null, "audio_tokens": 0,
                                       "image_tokens": null, "video_tokens": null,
                                       "web_search_requests": null,
                                       "google_maps_grounding_requests": null,
                                       "tool_use_tokens": null, "character_count": null,
                                       "image_count": null, "video_length_seconds": null,
                                       "audio_length_seconds": null, "query_count": null,
                                       "cache_write_tokens": null, "cached_tokens_details": null},
             "completion_tokens_details": {"reasoning_tokens": 4, "text_tokens": 3,
                                           "audio_tokens": null, "image_tokens": null,
                                           "video_tokens": null,
                                           "accepted_prediction_tokens": null,
                                           "rejected_prediction_tokens": null},
             "requests": 5}
        ]}"#;

#[test]
fn cost_extra_engine_combines_usage_and_keeps_the_cache_aliases() {
    let combined_request: CombineUsageRequest = request(COMBINED_USAGE_REQUEST);
    let combined = combine_usage(&combined_request).expect("the usages combine");
    assert_eq!(combined.prompt_tokens, 30);
    assert_eq!(combined.completion_tokens, 12);
    assert_eq!(combined.total_tokens, 42);
    let details = combined
        .prompt_tokens_details
        .expect("the details are kept");
    assert_eq!(details.text_tokens, Some(25));
    assert_eq!(details.cached_tokens, Some(5));
    assert_eq!(details.cache_creation_tokens, Some(3));
    assert_eq!(details.cache_write_tokens, Some(3));
    assert_eq!(details.audio_tokens, Some(0));
    let completion = combined
        .completion_tokens_details
        .expect("the completion details are kept");
    assert_eq!(completion.reasoning_tokens, Some(4));
    assert_eq!(combined.numeric_extras.len(), 1);

    let empty: CombineUsageRequest = request(r#"{"usages": []}"#);
    let defaulted = combine_usage(&empty).expect("an empty list combines to zero");
    assert_eq!(defaulted.prompt_tokens, 0);
    assert!(defaulted.prompt_tokens_details.is_none());
}

#[test]
fn cost_extra_engine_collects_and_partitions_realtime_usage() {
    let realtime: CollectUsageRequest = request(
        r#"{"source": "realtime", "results": [
            {"type": "session.created", "session": {"model": "realtime-test"}},
            {"type": "response.done", "response": {"usage": {
                "input_tokens": 100, "output_tokens": 20, "total_tokens": 120,
                "server_tool_use": {"web_search_requests": 2}}}},
            {"type": "response.audio.delta", "delta": "x"}
        ]}"#,
    );
    let collected = collect_usage(&realtime).expect("the realtime events are read");
    assert_eq!(collected.len(), 1);
    assert_eq!(collected[0].prompt_tokens, 100);
    assert_eq!(collected[0].completion_tokens, 20);
    assert_eq!(collected[0].total_tokens, 120);
    let tool_use = collected[0]
        .server_tool_use
        .as_ref()
        .expect("the tool use survives the transform");
    assert_eq!(
        tool_use.get("web_search_requests"),
        Some(&Json::Number(serde_json::Number::from(2_i64)))
    );
}

#[test]
fn cost_extra_engine_collects_websocket_usage() {
    let websocket: CollectUsageRequest = request(
        r#"{"source": "responses_ws", "results": [
            {"type": "response.completed", "response": {"usage": {
                "input_tokens": 10, "output_tokens": 2, "total_tokens": 12,
                "server_tool_use": {"web_search_requests": 2}}}},
            {"type": "response.completed", "response": {"service_tier": "flex"}},
            {"type": "response.incomplete", "response": {"usage": {
                "input_tokens": 3, "output_tokens": 1, "total_tokens": 4}}}
        ]}"#,
    );
    let websocket_usage = collect_usage(&websocket).expect("the websocket events are read");
    assert_eq!(websocket_usage.len(), 2);
    assert_eq!(
        websocket_usage[0]
            .server_tool_use
            .as_ref()
            .expect("the websocket tool use survives")
            .get("web_search_requests"),
        Some(&Json::Number(serde_json::Number::from(2_i64)))
    );
}

#[test]
fn cost_extra_engine_partitions_websocket_usage_by_tier() {
    let partition: PartitionUsageRequest = request(
        r#"{"results": [
            {"type": "response.completed", "response": {"service_tier": "default", "usage": {}}},
            {"type": "response.completed", "response": {"service_tier": null, "usage": {}}},
            {"type": "response.completed", "response": {"service_tier": "default", "usage": {}}},
            {"type": "response.incomplete", "response": {"service_tier": "flex", "usage": {}}}
        ]}"#,
    );
    let tiers = partition_usage(&partition).expect("the events partition");
    let names: Vec<Option<&str>> = tiers.iter().map(|(tier, _)| tier.as_deref()).collect();
    assert_eq!(names, vec![Some("default"), None, Some("flex")]);
    assert_eq!(tiers[0].1.len(), 2);
}

const REALTIME_TOKENS_REQUEST: &str = r#"{"results": [],
    "combined_usage": {"prompt_tokens": 100, "completion_tokens": 20,
        "total_tokens": 120, "prompt_tokens_details": null,
        "completion_tokens_details": null},
    "custom_llm_provider": "openai", "litellm_model_name": "realtime-test",
    "data_residency": null, "custom_pricing_model": null, "base_pricing_model": null}"#;

#[test]
fn cost_extra_engine_prices_a_realtime_session() {
    let tokens: RealtimeCostWire = request(REALTIME_TOKENS_REQUEST);
    let mut clock = SystemTime::now;
    let (cost, breakdown) = engine()
        .realtime_cost(&tokens, &mut clock)
        .expect("the session is priced");
    assert_eq!(cost.0, dec("0.0014"));
    assert!(breakdown.is_some());
}

#[test]
fn cost_extra_engine_prices_from_the_clock_it_is_given() {
    let tokens: RealtimeCostWire = request(REALTIME_TOKENS_REQUEST);
    let mut fixed = || SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let (cost, _breakdown) = engine()
        .realtime_cost(&tokens, &mut fixed)
        .expect("the fixed clock prices the session");
    assert_eq!(cost.0, dec("0.0014"));

    let mut before_epoch = || SystemTime::UNIX_EPOCH - Duration::from_secs(1);
    let error = engine()
        .realtime_cost(&tokens, &mut before_epoch)
        .expect_err("a clock before the epoch is unreadable");
    assert!(matches!(error, RealtimeFailure::Clock(_)));
}

#[test]
fn cost_extra_engine_adds_the_realtime_transcription_charge() {
    let transcribed: RealtimeCostWire = request(
        r#"{"results": [
            {"type": "transcription_session.created", "session": {"audio": {"input":
                {"transcription": {"model": "transcribe-test"}}}}},
            {"type": "conversation.item.input_audio_transcription.completed",
             "usage": {"type": "duration", "seconds": 2.5}}
        ], "combined_usage": {"prompt_tokens": 100, "completion_tokens": 20,
            "total_tokens": 120, "prompt_tokens_details": null,
            "completion_tokens_details": null},
            "custom_llm_provider": "openai", "litellm_model_name": "realtime-test",
            "data_residency": null, "custom_pricing_model": null, "base_pricing_model": null}"#,
    );
    let mut clock = SystemTime::now;
    let (charge, _breakdown) = engine()
        .realtime_cost(&transcribed, &mut clock)
        .expect("the session and its transcription are priced");
    assert_eq!(charge.0, dec("0.00165"));
}

#[test]
fn cost_extra_engine_prices_realtime_transcription_alone() {
    let request: RealtimeTranscriptionWire = request(
        r#"{"results": [
            {"type": "session.created", "session": {"model": "transcribe-test"}},
            {"type": "conversation.item.input_audio_transcription.completed",
             "usage": {"type": "duration", "seconds": 2.5}}
        ], "custom_llm_provider": "openai", "litellm_model_name": "realtime-test",
        "custom_pricing_model": null}"#,
    );
    assert_eq!(
        engine()
            .realtime_transcription_cost(&request)
            .expect("the transcriptions are priced")
            .0,
        dec("0.00025")
    );
}

#[test]
fn cost_extra_engine_reports_a_missing_provider() {
    let request: RerankRequest =
        request(r#"{"model": "rerank-test", "custom_llm_provider": null, "billed_units": null}"#);
    let failure = engine().rerank_cost(&request);
    assert!(matches!(failure, Err(RerankFailure::Resolution { .. })));
}

#[test]
fn cost_extra_engine_rejects_a_scalar_price_entry() {
    let mut model_cost: IndexMap<String, Json> =
        parse(CATALOG).expect("the fixture catalog parses");
    model_cost.insert(
        "scalar-test".to_owned(),
        Json::Text("not a price object".to_owned()),
    );
    let failure = Settings::build(SettingsWire {
        model_cost,
        cost_discount_config: CostAdjustmentMap::new(),
        cost_margin_config: CostAdjustmentMap::new(),
    });
    assert!(matches!(failure, Err(SettingsError::ModelEntry { .. })));
}

#[test]
fn cost_extra_engine_reads_a_realtime_event_from_the_wire() {
    let event: Event =
        parse(r#"{"type": "session.created", "session": {"model": "realtime-test"}}"#)
            .expect("the event parses");
    assert_eq!(event.len(), 2);
}
