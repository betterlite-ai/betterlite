#![cfg(test)]

use std::str::FromStr as _;

use betterlite_core::cost::realtime::Event;
use pyo3::prelude::*;
use rust_decimal::Decimal;
use serde::Deserialize;

use super::NativeCostExtra;
use crate::cost::convert::Amount;
use crate::cost_extra::convert::usage::UsageWire;

const SETTINGS: &str = r#"{"model_cost": {
    "ocr-test": {
        "litellm_provider": "mistral",
        "mode": "ocr",
        "ocr_cost_per_page": 0.001,
        "annotation_cost_per_page": 0.002,
        "ocr_cost_per_page_batches": 0.0005
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
    "whisper-test": {
        "litellm_provider": "openai",
        "mode": "audio_transcription",
        "output_cost_per_second": 0.0001
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
    "batch-test": {
        "litellm_provider": "openai",
        "mode": "chat",
        "input_cost_per_token": 0.0000025,
        "output_cost_per_token": 0.00001
    },
    "realtime-test": {
        "litellm_provider": "openai",
        "mode": "realtime",
        "input_cost_per_token": 0.00001,
        "output_cost_per_token": 0.00002
    }
}}"#;

#[derive(Debug, Deserialize)]
struct BreakdownWire {
    total_cost: Option<Amount>,
}

// A collected usage comes back with the logging Usage's own fields, which the request wire does not carry.
#[derive(Debug, Deserialize)]
struct CollectedUsage {
    prompt_tokens: i64,
    server_tool_use: Option<CollectedToolUse>,
}

#[derive(Debug, Deserialize)]
struct CollectedToolUse {
    web_search_requests: Option<i64>,
}

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("the fixture decimal parses")
}

fn native() -> NativeCostExtra {
    NativeCostExtra::new(SETTINGS).expect("the fixture settings build")
}

fn pair(text: &str) -> Vec<Amount> {
    serde_json::from_str(text).expect("the pair is JSON")
}

fn cost(text: &str) -> Amount {
    serde_json::from_str(text).expect("the cost is JSON")
}

#[test]
fn cost_extra_native_registers_the_class_on_a_module() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        super::super::register(&module).unwrap();
        assert!(module.hasattr("NativeCostExtra").unwrap());
    });
}

#[test]
fn cost_extra_native_refuses_settings_text_that_is_not_json() {
    Python::initialize();
    Python::attach(|_py| {
        assert!(NativeCostExtra::new("not json").is_err());
    });
}

#[test]
fn cost_extra_native_prices_the_ocr_calls() {
    Python::initialize();
    Python::attach(|_py| {
        let engine = native();
        let pages = engine
            .ocr_cost(
                r#"{"model": "ocr-test", "custom_llm_provider": "mistral",
                    "usage": {"pages_processed": 5, "pages_processed_annotation": 2},
                    "model_info": null}"#,
            )
            .expect("the pages are priced");
        assert_eq!(
            pair(&pages),
            vec![Amount(dec("0.009")), Amount(Decimal::ZERO)]
        );
        let batch = engine
            .ocr_batch_cost(
                r#"{"model": "ocr-test", "custom_llm_provider": "mistral",
                    "usage": {"pages_processed": 4, "pages_processed_annotation": 2},
                    "model_info": null}"#,
            )
            .expect("the batch pages are priced");
        assert_eq!(
            pair(&batch),
            vec![Amount(dec("0.006")), Amount(Decimal::ZERO)]
        );
    });
}

#[test]
fn cost_extra_native_prices_the_search_and_rerank_calls() {
    Python::initialize();
    Python::attach(|py| {
        let engine = native();
        let search = engine
            .vector_store_cost(
                py,
                r#"{"model": "vertex_ai/search_api", "custom_llm_provider": "vertex_ai",
                    "response": {}}"#,
            )
            .expect("the search is priced");
        assert_eq!(
            pair(&search),
            vec![Amount(dec("0.0015")), Amount(Decimal::ZERO)]
        );
        let rerank = engine
            .rerank_cost(
                py,
                r#"{"model": "rerank-test", "custom_llm_provider": "cohere",
                    "billed_units": {"search_units": 2}}"#,
            )
            .expect("the units are priced");
        assert_eq!(
            pair(&rerank),
            vec![Amount(dec("0.004")), Amount(Decimal::ZERO)]
        );
    });
}

#[test]
fn cost_extra_native_refuses_a_vector_provider_outside_llm_providers() {
    Python::initialize();
    Python::attach(|py| {
        let error = native()
            .vector_store_cost(
                py,
                r#"{"model": null, "custom_llm_provider": "typo-provider", "response": {}}"#,
            )
            .expect_err("a provider LlmProviders does not name is refused");
        assert_eq!(
            error.to_string(),
            "ValueError: 'typo-provider' is not a valid LlmProviders"
        );
    });
}

#[test]
fn cost_extra_native_prices_the_duration_calls() {
    Python::initialize();
    Python::attach(|py| {
        let engine = native();
        let transcription = engine
            .transcription_cost(
                py,
                r#"{"model": "whisper-test", "custom_llm_provider": null, "duration": 2.5}"#,
            )
            .expect("the seconds are priced");
        assert_eq!(
            pair(&transcription),
            vec![Amount(Decimal::ZERO), Amount(dec("0.00025"))]
        );
        let realtime = engine
            .realtime_transcription_cost(
                py,
                r#"{"results": [
                    {"type": "session.created", "session": {"model": "transcribe-test"}},
                    {"type": "conversation.item.input_audio_transcription.completed",
                     "usage": {"type": "duration", "seconds": 2.5}}
                ], "custom_llm_provider": "openai", "litellm_model_name": "realtime-test",
                "custom_pricing_model": null}"#,
            )
            .expect("the transcription seconds are priced");
        assert_eq!(cost(&realtime), Amount(dec("0.00025")));
    });
}

#[test]
fn cost_extra_native_prices_the_media_calls() {
    Python::initialize();
    Python::attach(|py| {
        let engine = native();
        let image = engine
            .image_cost(
                py,
                r#"{"model": "image-test", "custom_llm_provider": null, "quality": null, "n": 2,
                    "size": "1024-x-1024", "model_info": null}"#,
            )
            .expect("the images are priced");
        assert_eq!(cost(&image), Amount(dec("1")));
        let video = engine
            .video_cost(
                py,
                r#"{"model": "video-test", "duration_seconds": 8,
                    "custom_llm_provider": "vertex_ai", "model_info": null,
                    "video_resolution": "720p"}"#,
            )
            .expect("the video is priced");
        assert_eq!(cost(&video), Amount(dec("3.2")));
    });
}

#[test]
fn cost_extra_native_prices_the_batch_call() {
    Python::initialize();
    Python::attach(|py| {
        let text = native()
            .batch_cost(
                py,
                r#"{"usage": {"prompt_tokens": 1000, "completion_tokens": 500,
                    "total_tokens": 1500, "prompt_tokens_details": null,
                    "completion_tokens_details": null},
                    "model": "batch-test", "custom_llm_provider": "openai",
                    "model_info": null, "data_residency": null}"#,
            )
            .expect("the batch is priced");
        assert_eq!(
            pair(&text),
            vec![Amount(dec("0.00125")), Amount(dec("0.0025"))]
        );
    });
}

#[test]
fn cost_extra_native_answers_the_realtime_session_with_its_breakdown() {
    Python::initialize();
    Python::attach(|py| {
        let text = native()
            .realtime_cost(
                py,
                r#"{"results": [], "combined_usage": {"prompt_tokens": 100,
                    "completion_tokens": 20, "total_tokens": 120,
                    "prompt_tokens_details": null, "completion_tokens_details": null},
                    "custom_llm_provider": "openai", "litellm_model_name": "realtime-test",
                    "data_residency": null, "custom_pricing_model": null,
                    "base_pricing_model": null}"#,
            )
            .expect("the session is priced");
        let answer: (Amount, Option<BreakdownWire>) =
            serde_json::from_str(&text).expect("the answer is JSON");
        assert_eq!(answer.0, Amount(dec("0.0014")));
        let breakdown = answer.1.expect("the breakdown comes back");
        assert_eq!(breakdown.total_cost, Some(Amount(dec("0.0014"))));
    });
}

#[test]
fn cost_extra_native_combines_usage() {
    Python::initialize();
    Python::attach(|py| {
        let combined = NativeCostExtra::combine_usage(
            py,
            r#"{"usages": [
                {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15,
                 "prompt_tokens_details": null, "completion_tokens_details": null},
                {"prompt_tokens": 20, "completion_tokens": 7, "total_tokens": 27,
                 "prompt_tokens_details": null, "completion_tokens_details": null}
            ]}"#,
        )
        .expect("the usages combine");
        let usage: UsageWire = serde_json::from_str(&combined).expect("the usage is JSON");
        assert_eq!(usage.prompt_tokens, 30);
        assert_eq!(usage.completion_tokens, 12);
        assert_eq!(usage.total_tokens, 42);
    });
}

#[test]
fn cost_extra_native_collects_and_partitions_usage() {
    Python::initialize();
    Python::attach(|py| {
        let collected = NativeCostExtra::collect_usage(
            py,
            r#"{"source": "realtime", "results": [
                {"type": "session.created", "session": {"model": "realtime-test"}},
                {"type": "response.done", "response": {"usage": {
                    "input_tokens": 100, "output_tokens": 20, "total_tokens": 120,
                    "server_tool_use": {"web_search_requests": 2}}}}
            ]}"#,
        )
        .expect("the events collect");
        let usages: Vec<CollectedUsage> =
            serde_json::from_str(&collected).expect("the usages are JSON");
        assert_eq!(usages.len(), 1);
        assert_eq!(usages[0].prompt_tokens, 100);
        assert_eq!(
            usages[0]
                .server_tool_use
                .as_ref()
                .and_then(|tool_use| tool_use.web_search_requests),
            Some(2)
        );

        let partitioned = NativeCostExtra::partition_usage(
            py,
            r#"{"results": [
                {"type": "response.completed", "response": {"service_tier": "default", "usage": {}}},
                {"type": "response.completed", "response": {"service_tier": null, "usage": {}}}
            ]}"#,
        )
        .expect("the events partition");
        let tiers: Vec<(Option<String>, Vec<Event>)> =
            serde_json::from_str(&partitioned).expect("the partition is JSON");
        let names: Vec<Option<&str>> = tiers.iter().map(|(tier, _)| tier.as_deref()).collect();
        assert_eq!(names, vec![Some("default"), None]);
    });
}

fn value_error(answer: PyResult<String>) {
    let error = answer.expect_err("malformed text fails");
    assert!(error.to_string().starts_with("ValueError:"), "{error}");
}

#[test]
fn cost_extra_native_refuses_request_text_that_is_not_json() {
    Python::initialize();
    Python::attach(|py| {
        let engine = native();
        value_error(engine.ocr_cost("not json"));
        value_error(engine.ocr_batch_cost("not json"));
        value_error(engine.vector_store_cost(py, "not json"));
        value_error(engine.rerank_cost(py, "not json"));
        value_error(engine.transcription_cost(py, "not json"));
        value_error(engine.image_cost(py, "not json"));
        value_error(engine.video_cost(py, "not json"));
        value_error(engine.batch_cost(py, "not json"));
        value_error(NativeCostExtra::combine_usage(py, "not json"));
        value_error(NativeCostExtra::collect_usage(py, "not json"));
        value_error(NativeCostExtra::partition_usage(py, "not json"));
        value_error(engine.realtime_cost(py, "not json"));
        value_error(engine.realtime_transcription_cost(py, "not json"));
    });
}
