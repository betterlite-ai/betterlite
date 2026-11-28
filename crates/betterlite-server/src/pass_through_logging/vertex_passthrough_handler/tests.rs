#![cfg(test)]

use std::sync::Arc;

use betterlite_core::routes::CallType;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::UsageNumber;
use betterlite_openai::openai_video_identifiers::ids::decode_video_id_with_provider;
use betterlite_vertex::vertex_discovery_search::error::VertexDiscoverySearchError;
use betterlite_vertex::vertex_relay_metadata::types::CustomLlmProvider;
use http::{HeaderMap, HeaderValue};
use rstest::rstest;
use rust_decimal::Decimal;

use super::raw_predict::{completion_response, dummy_messages};
use super::{VertexBranch, VertexPassthroughError, select_vertex_branch};
use crate::pass_through_logging::gemini_passthrough_logging::reply::extra_json;
use crate::pass_through_logging::gemini_passthrough_logging::tests::support::{
    FakeCost, as_text, detail, field, integer, json, kwarg, kwargs, logged_model, logged_provider,
    number_text, optional_param, result_json, text,
};
use support::{
    METADATA_LISTS, capture_logs, fixture, handler, handler_with_predict, input, items,
    with_request_body,
};

mod batch_prediction_jobs;
mod support;

const INTERACTIONS_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/interactions";
const VIDEO_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/veo-3.0-generate-001:predictLongRunning";
const GENERATE_ROUTE: &str =
    "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent";
const GENERATE_VERTEX_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.0-flash:generateContent";
const EMBED_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/text-embedding-004:embedContent";
const BATCH_EMBED_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/text-embedding-004:batchEmbedContents";
const PREDICT_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/text-embedding-004:predict";
const RAW_PREDICT_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/anthropic/models/claude-3-sonnet-20240229:rawPredict";
const STREAM_RAW_PREDICT_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/anthropic/models/claude-3-sonnet-20240229:streamRawPredict";
const RAW_PREDICT_GOOGLE_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-pro:rawPredict";
const SEARCH_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/collections/c/dataStores/d/servingConfigs/default:search";
const BATCH_ROUTE: &str = "/vertex_ai/v1/projects/p1/locations/us-central1/batchPredictionJobs";
const NO_LOCATION_ROUTE: &str =
    "https://aiplatform.googleapis.com/v1/projects/proj/models/gemini-2.0-flash:countTokens";
const NONE_ROUTE: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-2.0-flash:countTokens";

const GENERATE_BODY: &str = r#"{"candidates":[{"content":{"parts":[{"text":"hello"}],"role":"model"},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2,"totalTokenCount":5},"modelVersion":"gemini-2.0-flash"}"#;
const VIDEO_BODY: &str = r#"{"name":"operations/generate_1234567890"}"#;
const CLAUDE_BODY: &str = r#"{"model":"claude-3-sonnet-20240229","content":[{"type":"text","text":"world"}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":1}}"#;
const INTERACTIONS_BODY: &str = r#"{"model":"response-model","usage":{"total_input_tokens":3,"total_output_tokens":2,"total_tokens":5}}"#;
const EMBED_BODY: &str = r#"{"embedding":{"values":[0.5,0.25]}}"#;
const EMBED_REQUEST: &str =
    r#"{"content":{"parts":[{"text":"hello"},{"inlineData":{"data":"x"}},{"text":"world"}]}}"#;
const BATCH_EMBED_BODY: &str = r#"{"embeddings":[{"values":[0.5]},{"values":[1.5,2.5]}]}"#;
const BATCH_EMBED_REQUEST: &str = r#"{"requests":[{"content":{"parts":[{"text":"ab"}]}},{"content":{"parts":[{"text":"c"},{"text":"d"}]}}]}"#;
const TEXT_BODY: &str = r#"{"predictions":[{"embeddings":{"values":[0.5,0.25],"statistics":{"truncated":false,"token_count":2}}},{"embeddings":{"values":[1.5],"statistics":{"truncated":false,"token_count":3}}}]}"#;
const BATCH_BODY: &str = r#"{"name":"projects/123/locations/us-central1/batchPredictionJobs/3814889423749775360","state":"JOB_STATE_PENDING","createTime":"2024-01-01T00:00:00.000000Z","inputConfig":{"gcsSource":{"uris":["gs://bucket/in.jsonl"]}},"model":"publishers/google/models/gemini-2.5-flash"}"#;
const SEARCH_BODY: &str = r#"{"results":[]}"#;
const BATCH_ID: &str = "3814889423749775360";
const BATCH_MODEL_PATH: &str = "publishers/google/models/gemini-2.5-flash";
const CREATED: i64 = 1_704_067_200;

#[rstest]
#[case::interactions(INTERACTIONS_ROUTE, Some(VertexBranch::Interactions))]
#[case::video(VIDEO_ROUTE, Some(VertexBranch::Video))]
#[case::generate_content(GENERATE_ROUTE, Some(VertexBranch::GenerateContent))]
#[case::embed_content(EMBED_ROUTE, Some(VertexBranch::EmbedContent))]
#[case::predict(PREDICT_ROUTE, Some(VertexBranch::Predict))]
#[case::raw_predict(RAW_PREDICT_ROUTE, Some(VertexBranch::RawPredict))]
#[case::search(SEARCH_ROUTE, Some(VertexBranch::Search))]
#[case::batch(BATCH_ROUTE, Some(VertexBranch::BatchPredictionJobs))]
#[case::none(NONE_ROUTE, None)]
fn order_selects_the_first_matching_branch(
    #[case] route: &str,
    #[case] expected: Option<VertexBranch>,
) {
    assert_eq!(select_vertex_branch(route), expected);
}

#[test]
fn order_predict_long_running_wins_over_predict() {
    let route = "https://host/v1/projects/p/locations/l/publishers/google/models/predict-veo:predictLongRunning";
    assert_eq!(select_vertex_branch(route), Some(VertexBranch::Video));
}

#[test]
fn order_stream_generate_content_matches_nothing() {
    let route =
        "https://host/v1/projects/p/locations/l/publishers/google/models/m:streamGenerateContent";
    assert_eq!(select_vertex_branch(route), None);
}

#[test]
fn order_embed_content_wins_over_predict() {
    let route = "https://host/v1/projects/p/locations/l/publishers/google/models/predict-embed:embedContent";
    assert_eq!(
        select_vertex_branch(route),
        Some(VertexBranch::EmbedContent)
    );
}

#[test]
fn order_batch_embed_contents_wins_over_predict() {
    let route = "https://host/v1/projects/p/locations/l/publishers/google/models/predict-embed:batchEmbedContents";
    assert_eq!(
        select_vertex_branch(route),
        Some(VertexBranch::EmbedContent)
    );
}

#[test]
fn order_raw_predict_is_not_predict() {
    let route = "https://host/v1/projects/p/locations/l/publishers/google/models/m:rawPredict";
    assert_eq!(select_vertex_branch(route), Some(VertexBranch::RawPredict));
}

#[test]
fn order_generate_content_wins_over_search_in_the_project_name() {
    let route = "https://host/v1/projects/search-project/locations/l/publishers/google/models/gemini-2.0-flash:generateContent";
    assert_eq!(
        select_vertex_branch(route),
        Some(VertexBranch::GenerateContent)
    );
}

#[test]
fn order_search_wins_over_batch_prediction_jobs() {
    let route = "/vertex_ai/v1/projects/search-project/locations/l/batchPredictionJobs";
    assert_eq!(select_vertex_branch(route), Some(VertexBranch::Search));
}

#[test]
fn order_interactions_comes_first() {
    let route = "https://host/v1/projects/search/locations/l/interactions";
    assert_eq!(
        select_vertex_branch(route),
        Some(VertexBranch::Interactions)
    );
}

#[rstest]
#[case::interactions(INTERACTIONS_ROUTE, INTERACTIONS_BODY)]
#[case::video(VIDEO_ROUTE, VIDEO_BODY)]
#[case::generate_content(GENERATE_VERTEX_ROUTE, GENERATE_BODY)]
#[case::embed_content(EMBED_ROUTE, EMBED_BODY)]
#[case::predict(PREDICT_ROUTE, TEXT_BODY)]
#[case::raw_predict(RAW_PREDICT_GOOGLE_ROUTE, "{}")]
#[case::search(SEARCH_ROUTE, SEARCH_BODY)]
#[case::batch_prediction_jobs(BATCH_ROUTE, "{}")]
#[case::none(NONE_ROUTE, "{}")]
fn location_is_written_to_optional_params_for_every_branch(
    #[case] route: &str,
    #[case] body: &str,
) {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(route, body);
    let _outcome = passthrough.vertex_passthrough_handler(input(&mut fixture));
    assert_eq!(
        optional_param(&fixture.logging_obj, "vertex_location"),
        Some(Json::Text("us-central1".to_owned()))
    );
}

#[test]
fn location_is_not_written_without_one() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(NO_LOCATION_ROUTE, "{}");
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the countTokens route matches no branch");
    assert!(outcome.result.is_none());
    assert_eq!(
        optional_param(&fixture.logging_obj, "vertex_location"),
        None
    );
}

#[test]
fn interactions_runs_the_b3638_handler_with_the_vertex_provider() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(INTERACTIONS_ROUTE, INTERACTIONS_BODY);
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the interactions reply is logged");
    let result = result_json(&outcome);
    assert_eq!(text(&result, "id"), "call-1");
    assert_eq!(text(&result, "model"), "response-model");
    let usage = field(&result, "usage");
    assert_eq!(integer(usage, "prompt_tokens"), 3);
    assert_eq!(integer(usage, "completion_tokens"), 2);
    assert_eq!(integer(usage, "total_tokens"), 5);
    assert_eq!(number_text(kwarg(&outcome, "response_cost")), "1.25");
    assert_eq!(as_text(kwarg(&outcome, "model")), "response-model");
    assert_eq!(as_text(kwarg(&outcome, "custom_llm_provider")), "vertex_ai");
    assert_eq!(logged_model(&fixture.logging_obj), "response-model");
    assert_eq!(
        logged_provider(&fixture.logging_obj).as_deref(),
        Some("vertex_ai")
    );
}

#[test]
fn interactions_prices_the_reply_with_the_vertex_location() {
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let (passthrough, _) = handler(Arc::clone(&cost));
    let mut fixture = fixture(INTERACTIONS_ROUTE, INTERACTIONS_BODY);
    passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the interactions reply is logged");
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "response-model");
    assert_eq!(calls[0].provider, "vertex_ai");
    assert_eq!(calls[0].vertex_location.as_deref(), Some("us-central1"));
    assert_eq!(calls[0].usage, Some((3, 2, 5)));
}

#[test]
fn interactions_uses_the_request_model_when_the_body_names_none() {
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let (passthrough, _) = handler(Arc::clone(&cost));
    let body = r#"{"usage":{"total_input_tokens":1,"total_output_tokens":1,"total_tokens":2}}"#;
    let mut fixture = with_request_body(
        fixture(INTERACTIONS_ROUTE, body),
        r#"{"model":"requested-model"}"#,
    );
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the interactions reply is logged");
    let result = result_json(&outcome);
    assert_eq!(text(&result, "model"), "requested-model");
    assert_eq!(cost.calls()[0].model, "requested-model");
}

#[test]
fn interactions_without_usage_keeps_the_caller_kwargs() {
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let (passthrough, _) = handler(Arc::clone(&cost));
    let mut fixture = fixture(INTERACTIONS_ROUTE, r#"{"model":"response-model"}"#);
    fixture.kwargs = kwargs(&[("extra", json("7"))]);
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("a reply without usage is not an error");
    assert!(outcome.result.is_none());
    assert_eq!(outcome.kwargs.entries.get("extra"), Some(&json("7")));
    assert!(cost.calls().is_empty());
}

#[test]
fn video_builds_the_operation_for_the_vertex_provider() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(VIDEO_ROUTE, VIDEO_BODY);
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the video operation is logged");
    let result = result_json(&outcome);
    assert_eq!(text(&result, "object"), "video");
    assert_eq!(text(&result, "status"), "processing");
    assert_eq!(text(&result, "model"), "veo-3.0-generate-001");
    let decoded = decode_video_id_with_provider(text(&result, "id"));
    assert_eq!(decoded.custom_llm_provider.as_deref(), Some("vertex_ai"));
    assert_eq!(decoded.model_id.as_deref(), Some("veo-3.0-generate-001"));
    assert_eq!(decoded.video_id, "operations/generate_1234567890");
}

#[test]
fn video_writes_the_logging_object_and_the_kwargs() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(VIDEO_ROUTE, VIDEO_BODY);
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the video operation is logged");
    assert_eq!(number_text(kwarg(&outcome, "response_cost")), "1.25");
    assert_eq!(as_text(kwarg(&outcome, "model")), "veo-3.0-generate-001");
    assert_eq!(as_text(kwarg(&outcome, "custom_llm_provider")), "vertex_ai");
    assert_eq!(logged_model(&fixture.logging_obj), "veo-3.0-generate-001");
    assert_eq!(
        logged_provider(&fixture.logging_obj).as_deref(),
        Some("vertex_ai")
    );
    assert_eq!(
        detail(&fixture.logging_obj, "model"),
        Some(Json::Text("veo-3.0-generate-001".to_owned()))
    );
    assert_eq!(
        detail(&fixture.logging_obj, "custom_llm_provider"),
        Some(Json::Text("vertex_ai".to_owned()))
    );
    let recorded = detail(&fixture.logging_obj, "response_cost").expect("the cost is written");
    assert_eq!(number_text(&recorded), "1.25");
}

#[test]
fn video_prices_the_create_call_with_the_location() {
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let (passthrough, _) = handler(Arc::clone(&cost));
    let mut fixture = fixture(VIDEO_ROUTE, VIDEO_BODY);
    passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the video operation is logged");
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "veo-3.0-generate-001");
    assert_eq!(calls[0].provider, "vertex_ai");
    assert_eq!(calls[0].call_type, CallType::CreateVideo);
    assert_eq!(calls[0].kind, "video");
    assert_eq!(
        calls[0].response_model.as_deref(),
        Some("veo-3.0-generate-001")
    );
    assert_eq!(calls[0].vertex_location.as_deref(), Some("us-central1"));
}

#[test]
fn video_passes_the_request_body() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = with_request_body(
        fixture(VIDEO_ROUTE, VIDEO_BODY),
        r#"{"parameters":{"resolution":" "}}"#,
    );
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the video operation is logged");
    let result = result_json(&outcome);
    assert_eq!(
        text(field(&result, "usage"), "video_resolution"),
        "",
        "the whitespace resolution survives as an empty one"
    );
}

#[rstest]
#[case::generate_content(GENERATE_ROUTE, "gemini")]
#[case::vertex_host(GENERATE_VERTEX_ROUTE, "vertex_ai")]
fn generate_decodes_the_reply_and_prices_it(#[case] route: &str, #[case] provider: &str) {
    let cost = FakeCost::priced(Decimal::new(25, 3));
    let (passthrough, _) = handler(Arc::clone(&cost));
    let mut fixture = fixture(route, GENERATE_BODY);
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the generateContent reply is logged");
    let result = result_json(&outcome);
    assert_eq!(text(&result, "id"), "call-1");
    assert_eq!(text(&result, "object"), "chat.completion");
    assert_eq!(text(&result, "model"), "gemini-2.0-flash");
    let choice = &items(field(&result, "choices"))[0];
    assert_eq!(text(choice, "finish_reason"), "stop");
    assert_eq!(integer(choice, "index"), 0);
    assert_eq!(text(field(choice, "message"), "content"), "hello");
    let usage = field(&result, "usage");
    assert_eq!(integer(usage, "prompt_tokens"), 3);
    assert_eq!(integer(usage, "completion_tokens"), 2);
    assert_eq!(integer(usage, "total_tokens"), 5);
    for name in METADATA_LISTS {
        assert_eq!(field(&result, name), &Json::List(Vec::new()));
    }
    assert_eq!(number_text(kwarg(&outcome, "response_cost")), "0.025");
    assert_eq!(as_text(kwarg(&outcome, "model")), "gemini-2.0-flash");
    assert_eq!(logged_model(&fixture.logging_obj), "gemini-2.0-flash");
    assert_eq!(
        detail(&fixture.logging_obj, "custom_llm_provider"),
        Some(Json::Text(provider.to_owned()))
    );
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "gemini-2.0-flash");
    assert_eq!(calls[0].provider, provider);
    assert_eq!(calls[0].kind, "completion");
    assert_eq!(calls[0].usage, Some((3, 2, 5)));
}

#[test]
fn generate_takes_the_location_from_the_route() {
    let cost = FakeCost::priced(Decimal::new(25, 3));
    let (passthrough, _) = handler(Arc::clone(&cost));
    let mut fixture = fixture(GENERATE_VERTEX_ROUTE, GENERATE_BODY);
    passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the generateContent reply is logged");
    assert_eq!(
        cost.calls()[0].vertex_location.as_deref(),
        Some("us-central1")
    );
}

#[test]
fn generate_keeps_the_caller_kwargs() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(25, 3)));
    let mut fixture = fixture(GENERATE_ROUTE, GENERATE_BODY);
    fixture.kwargs = kwargs(&[("litellm_params", json(r#"{"metadata":{}}"#))]);
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the generateContent reply is logged");
    assert_eq!(
        outcome.kwargs.entries.get("litellm_params"),
        Some(&json(r#"{"metadata":{}}"#))
    );
    assert_eq!(as_text(kwarg(&outcome, "model")), "gemini-2.0-flash");
}

#[rstest]
#[case::reasoning_content("reasoning_content")]
#[case::audio("audio")]
#[case::annotations("annotations")]
#[case::provider_specific_fields("provider_specific_fields")]
fn generate_drops_the_unset_message_fields(#[case] name: &str) {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(25, 3)));
    let mut fixture = fixture(GENERATE_ROUTE, GENERATE_BODY);
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the generateContent reply is logged");
    let result = result_json(&outcome);
    let choice = &items(field(&result, "choices"))[0];
    let Json::Object(message) = field(choice, "message") else {
        panic!("the message is an object");
    };
    assert!(
        message.get(name).is_none(),
        "{name} stays out of the message"
    );
}

#[test]
fn generate_usage_renders_numeric_extras_in_their_json_type() {
    assert_eq!(extra_json(&UsageNumber::Integer(7)), json("7"));
    assert_eq!(extra_json(&UsageNumber::Boolean(true)), json("true"));
}

#[test]
fn embed_passes_the_request_body_to_b3639() {
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let (passthrough, predict_cost) = handler(Arc::clone(&cost));
    let mut fixture = with_request_body(fixture(EMBED_ROUTE, EMBED_BODY), EMBED_REQUEST);
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the embedContent reply is logged");
    let result = result_json(&outcome);
    assert_eq!(text(&result, "object"), "list");
    assert_eq!(text(&result, "model"), "text-embedding-004");
    let usage = field(&result, "usage");
    assert_eq!(integer(usage, "prompt_tokens"), 3);
    assert_eq!(integer(usage, "total_tokens"), 3);
    let calls = predict_cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "text-embedding-004");
    assert_eq!(calls[0].provider, CustomLlmProvider::VertexAi);
    assert_eq!(calls[0].location.as_deref(), Some("us-central1"));
    assert_eq!(calls[0].usage, Some((3, 0, 3)));
    assert_eq!(number_text(kwarg(&outcome, "response_cost")), "1");
    assert_eq!(as_text(kwarg(&outcome, "custom_llm_provider")), "vertex_ai");
    assert_eq!(logged_model(&fixture.logging_obj), "text-embedding-004");
    assert!(cost.calls().is_empty());
}

#[test]
fn embed_joins_every_batch_request_part() {
    let (passthrough, predict_cost) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = with_request_body(
        fixture(BATCH_EMBED_ROUTE, BATCH_EMBED_BODY),
        BATCH_EMBED_REQUEST,
    );
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the batchEmbedContents reply is logged");
    let result = result_json(&outcome);
    assert_eq!(items(field(&result, "data")).len(), 2);
    let usage = field(&result, "usage");
    assert!(integer(usage, "prompt_tokens") > 0);
    assert!(predict_cost.calls()[0].usage.expect("usage").0 > 0);
}

#[test]
fn predict_does_not_pass_the_request_body() {
    let (passthrough, predict_cost) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = with_request_body(
        fixture(PREDICT_ROUTE, TEXT_BODY),
        r#"{"instances":[{"content":"ignored"}]}"#,
    );
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the predict reply is logged");
    let result = result_json(&outcome);
    assert_eq!(text(&result, "model"), "text-embedding-004");
    assert_eq!(items(field(&result, "data")).len(), 2);
    let usage = field(&result, "usage");
    assert_eq!(integer(usage, "prompt_tokens"), 5);
    assert_eq!(predict_cost.calls()[0].usage, Some((5, 0, 5)));
    assert_eq!(number_text(kwarg(&outcome, "response_cost")), "1");
}

#[test]
fn raw_predict_uses_the_anthropic_partner_config() {
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let (passthrough, _) = handler(Arc::clone(&cost));
    let mut fixture = fixture(RAW_PREDICT_ROUTE, CLAUDE_BODY);
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the partner reply is logged");
    let result = result_json(&outcome);
    assert_eq!(text(&result, "id"), "call-1");
    assert_eq!(text(&result, "model"), "claude-3-sonnet-20240229");
    let usage = field(&result, "usage");
    assert_eq!(integer(usage, "prompt_tokens"), 3);
    assert_eq!(integer(usage, "completion_tokens"), 1);
    assert_eq!(integer(usage, "total_tokens"), 4);
    let Json::Object(fields) = &result else {
        panic!("the chat response is an object");
    };
    assert!(!fields.contains_key("_hidden_params"));
    assert_eq!(number_text(kwarg(&outcome, "response_cost")), "1.25");
    assert_eq!(
        as_text(kwarg(&outcome, "model")),
        "vertex_ai/claude-3-sonnet-20240229"
    );
    assert_eq!(outcome.kwargs.entries.get("custom_llm_provider"), None);
    assert_eq!(
        logged_model(&fixture.logging_obj),
        "claude-3-sonnet-20240229"
    );
    assert_eq!(
        detail(&fixture.logging_obj, "custom_llm_provider"),
        Some(Json::Text("vertex_ai".to_owned()))
    );
}

#[test]
fn raw_predict_prices_the_partner_reply() {
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let (passthrough, _) = handler(Arc::clone(&cost));
    let mut fixture = fixture(RAW_PREDICT_ROUTE, CLAUDE_BODY);
    passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the partner reply is logged");
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "vertex_ai/claude-3-sonnet-20240229");
    assert_eq!(calls[0].provider, "vertex_ai");
    assert_eq!(calls[0].kind, "completion");
    assert_eq!(
        calls[0].response_model.as_deref(),
        Some("claude-3-sonnet-20240229")
    );
    assert_eq!(calls[0].usage, Some((3, 1, 4)));
    assert_eq!(calls[0].vertex_location.as_deref(), Some("us-central1"));
}

#[test]
fn raw_predict_without_a_publisher_returns_the_empty_chat_response() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(RAW_PREDICT_GOOGLE_ROUTE, "{}");
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("a body without a publisher is logged as an empty chat response");
    let result = result_json(&outcome);
    assert_eq!(text(&result, "id"), "call-1");
    assert_eq!(text(&result, "object"), "chat.completion");
    assert_eq!(integer(&result, "created"), CREATED);
    assert_eq!(field(&result, "model"), &Json::Null);
    let choice = &items(field(&result, "choices"))[0];
    assert_eq!(text(choice, "finish_reason"), "stop");
    assert_eq!(integer(choice, "index"), 0);
    let message = field(choice, "message");
    assert_eq!(text(message, "role"), "assistant");
    assert_eq!(field(message, "content"), &Json::Null);
    assert_eq!(field(message, "tool_calls"), &Json::Null);
    assert_eq!(field(message, "function_call"), &Json::Null);
    assert_eq!(as_text(kwarg(&outcome, "model")), "vertex_ai/gemini-pro");
    assert_eq!(logged_model(&fixture.logging_obj), "vertex_ai/gemini-pro");
}

#[test]
fn raw_predict_without_a_publisher_prices_the_bare_model() {
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let (passthrough, _) = handler(Arc::clone(&cost));
    let mut fixture = fixture(RAW_PREDICT_GOOGLE_ROUTE, "{}");
    passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("a body without a publisher is logged as an empty chat response");
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "vertex_ai/gemini-pro");
    assert_eq!(calls[0].response_model, None);
    assert_eq!(calls[0].usage, None);
}

#[test]
fn raw_predict_stream_raw_predict_takes_the_partner_branch() {
    let cost = FakeCost::priced(Decimal::new(125, 2));
    let (passthrough, _) = handler(Arc::clone(&cost));
    let mut fixture = fixture(STREAM_RAW_PREDICT_ROUTE, CLAUDE_BODY);
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the partner reply is logged");
    assert_eq!(
        as_text(kwarg(&outcome, "model")),
        "vertex_ai/claude-3-sonnet-20240229"
    );
    assert_eq!(cost.calls()[0].model, "vertex_ai/claude-3-sonnet-20240229");
}

#[test]
fn raw_predict_keeps_the_caller_kwargs() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(RAW_PREDICT_ROUTE, CLAUDE_BODY);
    fixture.kwargs = kwargs(&[("extra", json("true"))]);
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the partner reply is logged");
    assert_eq!(outcome.kwargs.entries.get("extra"), Some(&json("true")));
    assert_eq!(
        as_text(kwarg(&outcome, "model")),
        "vertex_ai/claude-3-sonnet-20240229"
    );
}

#[test]
fn search_wraps_the_reply() {
    let cost = FakeCost::priced(Decimal::new(3, 3));
    let (passthrough, _) = handler(Arc::clone(&cost));
    let mut fixture = fixture(SEARCH_ROUTE, SEARCH_BODY);
    fixture
        .call_log
        .model_call_details
        .insert("query".to_owned(), Json::Text("hello".to_owned()));
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the search reply is logged");
    assert!(matches!(
        outcome.result,
        Some(PassThroughResponse::VertexSearchWrapper(_))
    ));
    let result = result_json(&outcome);
    let response = field(&result, "response");
    assert_eq!(text(response, "object"), "vector_store.search_results.page");
    assert_eq!(text(response, "search_query"), "hello");
    assert!(items(field(response, "data")).is_empty());
    assert_eq!(number_text(kwarg(&outcome, "response_cost")), "0.003");
    assert_eq!(as_text(kwarg(&outcome, "model")), "vertex_ai/search_api");
    assert_eq!(outcome.kwargs.entries.get("custom_llm_provider"), None);
}

#[test]
fn search_writes_the_cost_and_the_base_model() {
    let cost = FakeCost::priced(Decimal::new(3, 3));
    let (passthrough, _) = handler(Arc::clone(&cost));
    let mut fixture = fixture(SEARCH_ROUTE, SEARCH_BODY);
    passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the search reply is logged");
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "vertex_ai/search_api");
    assert_eq!(calls[0].provider, "vertex_ai");
    assert_eq!(calls[0].call_type, CallType::VectorStoreSearch);
    assert_eq!(calls[0].vertex_location.as_deref(), Some("us-central1"));
    let details = &fixture.call_log.model_call_details;
    let params = details
        .get("litellm_params")
        .expect("litellm_params is created");
    assert_eq!(as_text(field(params, "base_model")), "vertex_ai/search_api");
    let recorded = details.get("response_cost").expect("the cost is written");
    assert_eq!(number_text(recorded), "0.003");
    assert!(fixture.call_log.model.is_empty());
    assert_eq!(fixture.call_log.custom_llm_provider, None);
}

#[test]
fn search_keeps_the_callers_litellm_params() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(3, 3)));
    let mut fixture = fixture(SEARCH_ROUTE, SEARCH_BODY);
    fixture
        .call_log
        .model_call_details
        .insert("litellm_params".to_owned(), json(r#"{"existing":"kept"}"#));
    passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the search reply is logged");
    let params = fixture
        .call_log
        .model_call_details
        .get("litellm_params")
        .expect("litellm_params stays");
    assert_eq!(text(params, "existing"), "kept");
    assert_eq!(text(params, "base_model"), "vertex_ai/search_api");
}

#[test]
fn search_list_litellm_params_is_an_error() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(3, 3)));
    let mut fixture = fixture(SEARCH_ROUTE, SEARCH_BODY);
    fixture
        .call_log
        .model_call_details
        .insert("litellm_params".to_owned(), json("[1]"));
    let error = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect_err("litellm_params must be an object");
    assert!(matches!(
        error,
        VertexPassthroughError::Detail {
            field: "litellm_params"
        }
    ));
}

#[test]
fn error_interactions_body_is_not_json() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(INTERACTIONS_ROUTE, "not json");
    let error = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect_err("a body that does not decode fails");
    assert!(matches!(error, VertexPassthroughError::Interactions(_)));
}

#[test]
fn error_video_body_is_not_an_operation() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(VIDEO_ROUTE, "not json");
    let error = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect_err("a body that does not decode fails");
    assert!(matches!(error, VertexPassthroughError::Video { .. }));
}

#[test]
fn error_generate_body_is_not_json() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(25, 3)));
    let mut fixture = fixture(GENERATE_ROUTE, "not json");
    let error = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect_err("a body that does not decode fails");
    assert!(matches!(error, VertexPassthroughError::Generate { .. }));
}

#[test]
fn error_embed_body_is_not_json() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(EMBED_ROUTE, "not json");
    let error = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect_err("a body that does not decode fails");
    assert!(matches!(error, VertexPassthroughError::Predict(_)));
}

#[test]
fn error_embed_cost_failure_rises() {
    let (passthrough, _) = handler_with_predict(FakeCost::priced(Decimal::new(125, 2)), None);
    let mut fixture = fixture(EMBED_ROUTE, EMBED_BODY);
    let error = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect_err("an unpriced embedding fails");
    assert!(matches!(error, VertexPassthroughError::Predict(_)));
}

#[test]
fn error_raw_predict_body_is_not_json() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(RAW_PREDICT_ROUTE, "not json");
    let error = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect_err("a body that does not decode fails");
    assert!(matches!(error, VertexPassthroughError::RawBody { .. }));
}

#[test]
fn error_raw_predict_parses_before_the_publisher() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(RAW_PREDICT_GOOGLE_ROUTE, "not json");
    let error = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect_err("the parse runs before the publisher is read");
    assert!(matches!(error, VertexPassthroughError::RawBody { .. }));
}

#[test]
fn error_raw_predict_partner_transform_fails() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(RAW_PREDICT_ROUTE, "[]");
    let error = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect_err("the partner transform fails on a non-object body");
    assert!(matches!(
        error,
        VertexPassthroughError::Partner {
            publisher: "anthropic",
            ..
        }
    ));
}

#[test]
fn error_search_body_is_not_an_object() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(3, 3)));
    let mut fixture = fixture(SEARCH_ROUTE, "[]");
    let mut headers = HeaderMap::new();
    headers.insert("x-search-id", HeaderValue::from_static("req-1"));
    fixture.headers = headers;
    let error = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect_err("a list is not a search page");
    let VertexPassthroughError::Search {
        source: VertexDiscoverySearchError::Provider(provider),
    } = error
    else {
        panic!("a body that does not decode is a provider error");
    };
    assert_eq!(provider.status_code, 200);
    assert_eq!(
        provider.headers.get("x-search-id"),
        Some(&Json::Text("req-1".to_owned()))
    );
}

#[test]
fn error_video_cost_failure_rises() {
    let (passthrough, _) = handler(FakeCost::unpriced());
    let mut fixture = fixture(VIDEO_ROUTE, VIDEO_BODY);
    let error = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect_err("an unpriced video fails");
    assert!(matches!(
        &error,
        VertexPassthroughError::Cost { model, .. } if model == "veo-3.0-generate-001"
    ));
}

#[test]
fn error_search_cost_failure_rises() {
    let (passthrough, _) = handler(FakeCost::unpriced());
    let mut fixture = fixture(SEARCH_ROUTE, SEARCH_BODY);
    let error = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect_err("an unpriced search fails");
    assert!(matches!(
        &error,
        VertexPassthroughError::Cost { model, .. } if model == "vertex_ai/search_api"
    ));
}

#[test]
fn raw_predict_dummy_message_is_the_litellm_message() {
    let messages = dummy_messages();
    assert_eq!(messages.len(), 1);
    let Json::Object(message) = &messages[0] else {
        panic!("the dummy message is an object");
    };
    assert_eq!(message.len(), 2);
    assert_eq!(message.get("role"), Some(&Json::Text("user".to_owned())));
    assert_eq!(
        message.get("content"),
        Some(&Json::Text("no-message-pass-through-endpoint".to_owned()))
    );
}

#[test]
fn raw_predict_completion_reads_the_body_model_and_usage() {
    let body = json(
        r#"{"model":"claude-3-sonnet-20240229","usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}"#,
    );
    let completion = completion_response(&body);
    let expected = Some("claude-3-sonnet-20240229");
    assert_eq!(completion.model.as_deref(), expected);
    let usage = completion.usage.expect("the usage is read");
    assert_eq!(usage.prompt_tokens, 3);
    assert_eq!(usage.completion_tokens, 1);
    assert_eq!(usage.total_tokens, 4);
    let empty = completion_response(&json(r#"{"model":""}"#));
    assert_eq!(empty.model, None);
    assert_eq!(empty.usage, None);
    let numeric_model = completion_response(&json(r#"{"model":7}"#));
    assert_eq!(numeric_model.model, None);
    let text_usage = completion_response(&json(r#"{"usage":"none"}"#));
    assert_eq!(text_usage.usage, None);
    let not_object = completion_response(&json("[]"));
    assert_eq!(not_object.model, None);
}

#[test]
fn raw_predict_writes_the_partner_debug_log() {
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(125, 2)));
    let mut fixture = fixture(RAW_PREDICT_ROUTE, CLAUDE_BODY);
    let (logs, _guard) = capture_logs();
    let outcome = passthrough.vertex_passthrough_handler(input(&mut fixture));
    outcome.expect("the reply is logged");
    let captured = logs.text();
    assert!(
        captured.contains("RAW RESPONSE"),
        "the partner transform's debug log is captured: {captured}"
    );
}

#[test]
fn generate_usage_details_use_the_litellm_names() {
    let body = r#"{"candidates":[{"content":{"parts":[{"text":"hello"}],"role":"model"},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":7,"candidatesTokenCount":2,"totalTokenCount":9,"cachedContentTokenCount":5,"promptTokensDetails":[{"modality":"TEXT","tokenCount":9},{"modality":"IMAGE","tokenCount":1}],"candidatesTokensDetails":[{"modality":"TEXT","tokenCount":2}]},"modelVersion":"gemini-2.0-flash"}"#;
    let (passthrough, _) = handler(FakeCost::priced(Decimal::new(25, 3)));
    let mut fixture = fixture(GENERATE_ROUTE, body);
    let outcome = passthrough
        .vertex_passthrough_handler(input(&mut fixture))
        .expect("the generateContent reply is logged");
    let result = result_json(&outcome);
    let usage = field(&result, "usage");
    assert_eq!(integer(usage, "prompt_tokens"), 7);
    let prompt = field(usage, "prompt_tokens_details");
    assert_eq!(integer(prompt, "text_tokens"), 4);
    assert_eq!(integer(prompt, "image_tokens"), 1);
    assert_eq!(integer(prompt, "cached_tokens"), 5);
    assert_eq!(integer(usage, "cache_read_input_tokens"), 5);
    let completion = field(usage, "completion_tokens_details");
    assert_eq!(integer(completion, "text_tokens"), 2);
}
