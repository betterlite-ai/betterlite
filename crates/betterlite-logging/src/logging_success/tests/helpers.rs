use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::support::{
    Fixture, bare_request, chat_result, chunk_result, correlation, json_object, payload, request,
    state, time,
};
use crate::logging_success::assembled::{
    ASYNC_STREAM_RESPONSE_KEY, COMPLETE_STREAMING_RESPONSE, claim_final_stream_dispatch,
    is_assembled_stream_success,
};
use crate::logging_success::handler::success_handler;
use crate::logging_success::payload_state::{STANDARD_LOGGING_OBJECT, standard_logging_object};
use crate::logging_success::prepare::prepare;
use crate::logging_success::recognized::{is_interactions_create, is_recognized_call_type};
use crate::logging_success::types::SuccessResult;

fn object_with(entries: &[(&str, Json)]) -> SuccessResult {
    SuccessResult::Other(json_object(entries))
}

#[rstest]
#[case::assembled_not_stream("assembled_not_stream")]
#[case::assembled_response("assembled_response")]
#[case::assembled_chunk("assembled_chunk")]
#[case::assembled_async_key("assembled_async_key")]
#[case::assembled_complete_null("assembled_complete_null")]
#[case::assembled_complete_value("assembled_complete_value")]
#[case::assembled_null_result("assembled_null_result")]
#[case::assembled_null_result_async_key("assembled_null_result_async_key")]
#[case::assembled_null_result_complete("assembled_null_result_complete")]
#[case::prepare_null_result("prepare_null_result")]
#[case::recognized_chat("recognized_chat")]
#[case::recognized_embedding("recognized_embedding")]
#[case::recognized_chunk("recognized_chunk")]
#[case::recognized_search_object("recognized_search_object")]
#[case::recognized_other_object("recognized_other_object")]
#[case::recognized_list("recognized_list")]
#[case::recognized_mcp("recognized_mcp")]
#[case::interactions_create("interactions_create")]
#[case::interactions_other("interactions_other")]
#[case::prepare_defaults("prepare_defaults")]
#[case::prepare_request_times("prepare_request_times")]
#[case::prepare_stream_seeds_cost("prepare_stream_seeds_cost")]
#[case::payload_malformed("payload_malformed")]
#[case::payload_unknown_field("payload_unknown_field")]
#[case::payload_round_trip("payload_round_trip")]
#[case::claim_once("claim_once")]
fn logging_success_behavior_tests_behavior(#[case] scenario: &str) {
    match scenario {
        "assembled_not_stream" => {
            let state = state(Some(false));
            assert!(!is_assembled_stream_success(&state, &chat_result()));
        }
        "assembled_response" => {
            let state = state(Some(true));
            assert!(is_assembled_stream_success(&state, &chat_result()));
        }
        "assembled_chunk" => {
            let state = state(Some(true));
            assert!(!is_assembled_stream_success(&state, &chunk_result()));
        }
        "assembled_async_key" => {
            let mut state = state(Some(true));
            state
                .model_call_details
                .insert(ASYNC_STREAM_RESPONSE_KEY.to_owned(), Json::Null);
            assert!(is_assembled_stream_success(&state, &chunk_result()));
        }
        "assembled_complete_null" => {
            let mut state = state(Some(true));
            state
                .model_call_details
                .insert(COMPLETE_STREAMING_RESPONSE.to_owned(), Json::Null);
            assert!(!is_assembled_stream_success(&state, &chunk_result()));
        }
        "assembled_complete_value" => {
            let mut state = state(Some(true));
            state.model_call_details.insert(
                COMPLETE_STREAMING_RESPONSE.to_owned(),
                Json::Object(IndexMap::new()),
            );
            assert!(is_assembled_stream_success(&state, &chunk_result()));
        }
        "recognized_chat" => assert!(is_recognized_call_type(&chat_result(), "completion")),
        "recognized_embedding" => assert!(is_recognized_call_type(
            &SuccessResult::Embeddings(json_object(&[])),
            "embedding"
        )),
        "recognized_chunk" => assert!(is_recognized_call_type(&chunk_result(), "completion")),
        "recognized_search_object" => assert!(is_recognized_call_type(
            &object_with(&[("object", Json::Text("search".to_owned()))]),
            "search"
        )),
        "recognized_other_object" => assert!(!is_recognized_call_type(
            &object_with(&[("object", Json::Text("chat.completion".to_owned()))]),
            "completion"
        )),
        "recognized_list" => assert!(!is_recognized_call_type(
            &SuccessResult::Other(Json::List(Vec::new())),
            "completion"
        )),
        "recognized_mcp" => assert!(is_recognized_call_type(
            &SuccessResult::Other(Json::Null),
            "call_mcp_tool"
        )),
        "interactions_create" => {
            assert!(is_interactions_create("create_interaction"));
            assert!(is_interactions_create("acreate"));
        }
        "interactions_other" => {
            assert!(!is_interactions_create("get_interaction"));
            assert!(!is_interactions_create("completion"));
        }
        "assembled_null_result" => {
            let state = state(Some(true));
            assert!(!is_assembled_stream_success(
                &state,
                &SuccessResult::Other(Json::Null)
            ));
        }
        "assembled_null_result_async_key" => {
            let mut state = state(Some(true));
            state
                .model_call_details
                .insert(ASYNC_STREAM_RESPONSE_KEY.to_owned(), Json::Null);
            assert!(is_assembled_stream_success(
                &state,
                &SuccessResult::Other(Json::Null)
            ));
        }
        "assembled_null_result_complete" => {
            let mut state = state(Some(true));
            state.model_call_details.insert(
                COMPLETE_STREAMING_RESPONSE.to_owned(),
                Json::Text("done".to_owned()),
            );
            assert!(is_assembled_stream_success(
                &state,
                &SuccessResult::Other(Json::Null)
            ));
        }
        "prepare_null_result" => {
            let fixture = Fixture::new();
            let mut state = state(Some(false));
            state.call_type = "call_mcp_tool".to_owned();
            let handler = fixture.handler();
            prepare(
                &handler,
                &mut state,
                &bare_request(SuccessResult::Other(Json::Null)),
            );
            assert_eq!(fixture.builder.builds(), 0);
            assert!(standard_logging_object(&state).unwrap().is_none());
            assert_eq!(
                state.model_call_details.get("response_cost"),
                Some(&Json::Null)
            );
        }
        "prepare_defaults" => {
            let fixture = Fixture::new();
            let mut state = state(Some(false));
            let handler = fixture.handler();
            let prepared = prepare(&handler, &mut state, &bare_request(chat_result()));
            assert_eq!(prepared.end_time, time("2024-01-01T00:00:02+00:00"));
            assert_eq!(
                state.completion_start_time,
                Some(time("2024-01-01T00:00:02+00:00"))
            );
            assert_eq!(
                state.model_call_details.get("log_event_type"),
                Some(&Json::Text("successful_api_call".to_owned()))
            );
            assert_eq!(state.model_call_details.get("cache_hit"), Some(&Json::Null));
            assert_eq!(fixture.builder.builds(), 1);
            assert!(standard_logging_object(&state).unwrap().is_some());
        }
        "prepare_request_times" => {
            let fixture = Fixture::new();
            let mut state = state(Some(false));
            let handler = fixture.handler();
            let request = request(chat_result());
            let prepared = prepare(&handler, &mut state, &request);
            assert_eq!(prepared.end_time, time("2024-01-01T00:00:01+00:00"));
            assert_eq!(
                state.model_call_details.get("end_time"),
                Some(&Json::Text("2024-01-01T00:00:01+00:00".to_owned()))
            );
            assert_eq!(
                state.model_call_details.get("cache_hit"),
                Some(&Json::Bool(false))
            );
            assert_eq!(fixture.builder.builds(), 0);
        }
        "prepare_stream_seeds_cost" => {
            let fixture = Fixture::new();
            let mut state = state(Some(true));
            let handler = fixture.handler();
            prepare(&handler, &mut state, &bare_request(chunk_result()));
            assert_eq!(
                state.model_call_details.get("response_cost"),
                Some(&Json::Null)
            );
            assert_eq!(fixture.builder.builds(), 0);
        }
        "payload_malformed" => {
            let mut state = state(Some(false));
            state.model_call_details.insert(
                STANDARD_LOGGING_OBJECT.to_owned(),
                Json::Text("not a payload".to_owned()),
            );
            standard_logging_object(&state).unwrap_err();
        }
        "payload_unknown_field" => {
            let mut state = state(Some(false));
            let mut fields = IndexMap::new();
            fields.insert("unknown_field".to_owned(), Json::Null);
            state
                .model_call_details
                .insert(STANDARD_LOGGING_OBJECT.to_owned(), Json::Object(fields));
            standard_logging_object(&state).unwrap_err();
        }
        "payload_round_trip" => {
            let fixture = Fixture::new();
            let mut state = state(Some(false));
            let handler = fixture.handler();
            success_handler(
                &handler,
                &mut state,
                &mut correlation(),
                &request(chat_result()),
            );
            let stored = standard_logging_object(&state).unwrap().unwrap();
            assert_eq!(stored.id, payload().id);
            assert_eq!(stored.messages, payload().messages);
        }
        "claim_once" => {
            let mut state = state(Some(false));
            assert!(claim_final_stream_dispatch(&mut state));
            assert!(!claim_final_stream_dispatch(&mut state));
        }
        other => panic!("unexpected scenario {other}"),
    }
}
