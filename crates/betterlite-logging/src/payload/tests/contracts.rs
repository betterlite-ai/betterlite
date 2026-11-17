use std::io::{BufReader, Read as _, Seek as _, SeekFrom, stdout};
use std::os::fd::AsFd as _;
use std::sync::Arc;

use betterlite_config::sections::logging_privacy::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use nix::unistd::{dup, dup2_stdout};
use rstest::rstest;

use super::behaviors::FixedClock;
use super::{
    BASE_PAYLOAD, builder_with, chat_response, end, init, payload_from, sources_with, start, state,
    state_from,
};
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_state::{LoggingInit, LoggingState};
use crate::logging_success::assembled::{ASYNC_STREAM_RESPONSE_KEY, COMPLETE_STREAMING_RESPONSE};
use crate::logging_success::ports::StandardPayloadBuilder;
use crate::payload::deps::{
    AbsentAutorouterAudit, AbsentModelCostInformation, AbsentRequestAccessGroups,
    AbsentSavedCacheCost, AutorouterAudit as _, MODEL_MAP_KEY, MODEL_MAP_VALUE,
    ModelCostInformation as _, ModelCostRequest, PayloadClock as _, PayloadSettings,
    PayloadSources, RequestAccessGroups as _, SavedCacheCost as _, SavingsRequest,
    SystemPayloadClock,
};
use crate::payload::emit::{EmitSettings, PayloadEmitter as _, StdoutEmitter};
use crate::payload::response::{object_field, object_or_empty, record_served_output_texts};

fn built_with(
    sources: PayloadSources,
    state: &LoggingState,
    response: &Json,
) -> StandardLoggingPayload {
    StandardPayloadBuilder::build(
        &builder_with(sources, PayloadSettings::default(), Settings::default()),
        state,
        response,
        start(),
        end(),
    )
    .expect("the payload builds")
}

fn built(configuration: LoggingInit, response: &Json) -> StandardLoggingPayload {
    let state = state_from(configuration);
    built_with(PayloadSources::default(), &state, response)
}

#[derive(Clone, Copy, Debug)]
enum ContractBehavior {
    DefaultSourcesAbsences,
    AbsentModelCostMap,
    AbsentSavedCacheCost,
    AbsentAccessGroups,
    AbsentAutorouterAudit,
    WallClockSeconds,
    ResponsesUsageNeedsConverter,
    CacheHitFractionalClock,
    CallIdSources,
    ParamsSupplyTraceId,
    StreamFlagSources,
    AzureEmptyStamp,
    AzureEmptyResponseModel,
    AzureRouterNames,
    NonRouterKeepsModel,
    DeploymentNamesModel,
    ModelInfoId,
    ResponseCostFields,
    StoredOverheadWins,
    UnbilledManagementCalls,
    ModelParameters,
    PassThroughFields,
    ServedTexts,
    RequestTags,
    ObjectOrEmpty,
    StdoutEmitterOutput,
    NonStreamTiming,
    NullRoutingDecision,
    ResponseIdFallback,
}

#[rstest]
#[case::default_sources_keep_explicit_absences(ContractBehavior::DefaultSourcesAbsences)]
#[case::absent_model_cost_map_carries_both_keys(ContractBehavior::AbsentModelCostMap)]
#[case::absent_saved_cache_cost_is_absent(ContractBehavior::AbsentSavedCacheCost)]
#[case::absent_access_groups_are_absent(ContractBehavior::AbsentAccessGroups)]
#[case::absent_autorouter_audit_is_absent(ContractBehavior::AbsentAutorouterAudit)]
#[case::system_clock_reads_epoch_seconds(ContractBehavior::WallClockSeconds)]
#[case::responses_usage_needs_the_converter(ContractBehavior::ResponsesUsageNeedsConverter)]
#[case::cache_hit_suffix_keeps_the_fraction(ContractBehavior::CacheHitFractionalClock)]
#[case::call_id_prefers_details_then_params(ContractBehavior::CallIdSources)]
#[case::litellm_params_supply_the_trace_id(ContractBehavior::ParamsSupplyTraceId)]
#[case::stream_needs_the_state_and_a_stored_response(ContractBehavior::StreamFlagSources)]
#[case::azure_empty_stamp_falls_back(ContractBehavior::AzureEmptyStamp)]
#[case::azure_empty_response_model_falls_back(ContractBehavior::AzureEmptyResponseModel)]
#[case::azure_router_names_keep_the_response_model(ContractBehavior::AzureRouterNames)]
#[case::non_router_keeps_the_requested_model(ContractBehavior::NonRouterKeepsModel)]
#[case::deployment_metadata_names_the_model(ContractBehavior::DeploymentNamesModel)]
#[case::model_info_id_reaches_the_payload(ContractBehavior::ModelInfoId)]
#[case::response_cost_reaches_the_hidden_params(ContractBehavior::ResponseCostFields)]
#[case::stored_overhead_is_not_overwritten(ContractBehavior::StoredOverheadWins)]
#[case::management_calls_are_unbilled(ContractBehavior::UnbilledManagementCalls)]
#[case::logged_model_parameters_are_kept(ContractBehavior::ModelParameters)]
#[case::cost_diagnostics_and_cache_key_pass_through(ContractBehavior::PassThroughFields)]
#[case::served_texts_replace_the_message_content(ContractBehavior::ServedTexts)]
#[case::request_tags_include_the_user_agent(ContractBehavior::RequestTags)]
#[case::object_or_empty_copies_only_objects(ContractBehavior::ObjectOrEmpty)]
#[case::stdout_emitter_prints_one_json_line(ContractBehavior::StdoutEmitterOutput)]
#[case::non_streamed_calls_end_at_the_end_time(ContractBehavior::NonStreamTiming)]
#[case::null_routing_decision_has_no_estimate(ContractBehavior::NullRoutingDecision)]
#[case::a_missing_response_id_falls_back_to_the_call_id(ContractBehavior::ResponseIdFallback)]
#[tokio::test]
async fn logging_payloads_logging_payload_tests_behavior(#[case] behavior: ContractBehavior) {
    match behavior {
        ContractBehavior::DefaultSourcesAbsences => {
            let state = state(None);
            let response = chat_response();
            let payload = built_with(PayloadSources::default(), &state, &response);
            assert_eq!(payload.id, "chatcmpl-1");
            assert_eq!(payload.model, "gpt-4o");
            assert_eq!(payload.litellm_call_id.as_deref(), Some("call-1"));
            assert_eq!(payload.stream, None);
            assert_eq!(payload.cache_hit, Some(false));
            assert_eq!(
                payload.model_map_information,
                IndexMap::from([
                    (MODEL_MAP_KEY.to_owned(), Json::Text(String::new())),
                    (MODEL_MAP_VALUE.to_owned(), Json::Null),
                ])
            );
            assert_eq!(payload.saved_cache_cost.to_string(), "0.0");
            assert_eq!(payload.request_model_access_groups, Some(Vec::new()));
            assert!(payload.hidden_params.response_cost.is_none());
            assert!(payload.autorouter_savings.is_none());
            assert!(payload.baseline_observation.is_none());
            assert!(payload.autorouter_savings_estimate.is_none());
            assert_eq!(
                payload.standard_built_in_tools_params,
                Some(Json::Object(IndexMap::from([
                    ("web_search_options".to_owned(), Json::Null),
                    ("file_search".to_owned(), Json::Null),
                ])))
            );
            assert_eq!(payload.response, Some(response));
        }
        ContractBehavior::AbsentModelCostMap => {
            let details = IndexMap::new();
            let response = Json::Null;
            let request = ModelCostRequest {
                model_call_details: &details,
                init_response_obj: &response,
                api_base: None,
                custom_llm_provider: None,
            };
            assert_eq!(
                AbsentModelCostInformation.model_map_information(&request),
                IndexMap::from([
                    (MODEL_MAP_KEY.to_owned(), Json::Text(String::new())),
                    (MODEL_MAP_VALUE.to_owned(), Json::Null),
                ])
            );
        }
        ContractBehavior::AbsentSavedCacheCost => {
            assert_eq!(AbsentSavedCacheCost.saved_cache_cost(&Json::Null), None);
        }
        ContractBehavior::AbsentAccessGroups => {
            assert_eq!(
                AbsentRequestAccessGroups.access_groups(&IndexMap::new()),
                None
            );
        }
        ContractBehavior::AbsentAutorouterAudit => {
            let metadata = IndexMap::new();
            let usage = Json::Null;
            let request = SavingsRequest {
                metadata: &metadata,
                model: "gpt-4o",
                custom_llm_provider: None,
                model_id: "",
                usage_object: &usage,
                cost_breakdown: None,
            };
            assert_eq!(AbsentAutorouterAudit.savings(&request), None);
            assert_eq!(AbsentAutorouterAudit.baseline_observation(), None);
            assert!(!AbsentAutorouterAudit.baseline_context_active());
        }
        ContractBehavior::WallClockSeconds => {
            let now = SystemPayloadClock.now_epoch_seconds();
            assert!(
                (1_700_000_000.0_f64..10_000_000_000.0_f64).contains(&now),
                "the wall clock reads epoch seconds, got {now}"
            );
        }
        ContractBehavior::ResponsesUsageNeedsConverter => {
            let response: Json = serde_json::from_str(
                r#"{"id": "resp-1", "usage": {"input_tokens": 3, "output_tokens": 4, "total_tokens": 7}}"#,
            )
            .unwrap();
            let state = state(None);
            let payload = StandardPayloadBuilder::build(
                &builder_with(
                    PayloadSources::default(),
                    PayloadSettings::default(),
                    Settings::default(),
                ),
                &state,
                &response,
                start(),
                end(),
            );
            assert!(
                payload.is_none(),
                "a Responses usage object needs the injected conversion"
            );
        }
        ContractBehavior::CacheHitFractionalClock => {
            let mut state = state(None);
            state
                .model_call_details
                .insert("cache_hit".to_owned(), Json::Bool(true));
            let sources = sources_with(
                Arc::new(FixedClock(1234.5)),
                Arc::new(AbsentSavedCacheCost),
                Arc::new(AbsentAutorouterAudit),
            );
            let payload = built_with(sources, &state, &chat_response());
            assert_eq!(payload.id, "chatcmpl-1_cache_hit1234.5");
            assert_eq!(payload.cache_hit, Some(true));
        }
        ContractBehavior::CallIdSources => {
            let mut from_details = state(None);
            from_details.model_call_details.insert(
                "litellm_call_id".to_owned(),
                Json::Text("kwargs-call".to_owned()),
            );
            let details_payload =
                built_with(PayloadSources::default(), &from_details, &chat_response());
            assert_eq!(
                details_payload.litellm_call_id.as_deref(),
                Some("kwargs-call")
            );
            let mut from_params = state(None);
            from_params
                .model_call_details
                .insert("litellm_call_id".to_owned(), Json::Text(String::new()));
            from_params.litellm_params.insert(
                "litellm_call_id".to_owned(),
                Json::Text("params-call".to_owned()),
            );
            let params_payload =
                built_with(PayloadSources::default(), &from_params, &chat_response());
            assert_eq!(
                params_payload.litellm_call_id.as_deref(),
                Some("params-call")
            );
        }
        ContractBehavior::ParamsSupplyTraceId => {
            let mut configuration = init(None);
            configuration.litellm_params.insert(
                "litellm_trace_id".to_owned(),
                Json::Text("params-trace".to_owned()),
            );
            let payload = built(configuration, &chat_response());
            assert_eq!(payload.trace_id, "params-trace");
        }
        ContractBehavior::StreamFlagSources => {
            let mut unstreamed = state(None);
            unstreamed
                .model_call_details
                .insert(COMPLETE_STREAMING_RESPONSE.to_owned(), Json::Bool(true));
            let unstreamed_payload =
                built_with(PayloadSources::default(), &unstreamed, &chat_response());
            assert_eq!(unstreamed_payload.stream, None);
            let mut streamed = state(Some(true));
            streamed
                .model_call_details
                .insert(ASYNC_STREAM_RESPONSE_KEY.to_owned(), Json::Bool(true));
            let streamed_payload =
                built_with(PayloadSources::default(), &streamed, &chat_response());
            assert_eq!(streamed_payload.stream, Some(true));
            let mut nulled = state(Some(true));
            nulled
                .model_call_details
                .insert(COMPLETE_STREAMING_RESPONSE.to_owned(), Json::Null);
            let nulled_payload = built_with(PayloadSources::default(), &nulled, &chat_response());
            assert_eq!(nulled_payload.stream, None);
        }
        ContractBehavior::AzureEmptyStamp => {
            let response: Json = serde_json::from_str(
                r#"{"id": "chatcmpl-1", "model": "served-model", "choices": [],
                    "_hidden_params": {"azure_model_router_selected_model": ""}}"#,
            )
            .unwrap();
            let mut configuration = init(None);
            configuration.model = "gpt-4o".to_owned();
            let payload = built(configuration, &response);
            assert_eq!(payload.model, "gpt-4o");
        }
        ContractBehavior::AzureEmptyResponseModel => {
            let response: Json =
                serde_json::from_str(r#"{"id": "chatcmpl-1", "model": "", "choices": []}"#)
                    .unwrap();
            let mut configuration = init(None);
            configuration.model = "model_router/gpt-4o".to_owned();
            let payload = built(configuration, &response);
            assert_eq!(payload.model, "model_router/gpt-4o");
        }
        ContractBehavior::AzureRouterNames => {
            let response: Json = serde_json::from_str(
                r#"{"id": "chatcmpl-1", "model": "served-by-router", "choices": [],
                    "_hidden_params": {"litellm_model_name": "model_router/gpt-4o"}}"#,
            )
            .unwrap();
            let mut configuration = init(None);
            configuration.model = "model_router/gpt-4o".to_owned();
            let payload = built(configuration, &response);
            assert_eq!(payload.model, "served-by-router");
        }
        ContractBehavior::NonRouterKeepsModel => {
            let response: Json = serde_json::from_str(
                r#"{"id": "chatcmpl-1", "model": "served-model", "choices": []}"#,
            )
            .unwrap();
            let mut configuration = init(None);
            configuration.model = "gpt-4o".to_owned();
            let payload = built(configuration, &response);
            assert_eq!(payload.model, "gpt-4o");
        }
        ContractBehavior::DeploymentNamesModel => {
            let mut configuration = init(None);
            configuration.litellm_params.insert(
                "metadata".to_owned(),
                Json::Object(IndexMap::from([(
                    "deployment".to_owned(),
                    Json::Text("azure/gpt-4o".to_owned()),
                )])),
            );
            let payload = built(configuration, &chat_response());
            assert_eq!(payload.model, "azure/gpt-4o");
        }
        ContractBehavior::ModelInfoId => {
            let mut configuration = init(None);
            configuration.litellm_params.insert(
                "metadata".to_owned(),
                Json::Object(IndexMap::from([
                    (
                        "model_info".to_owned(),
                        Json::Object(IndexMap::from([(
                            "id".to_owned(),
                            Json::Text("model-id-1".to_owned()),
                        )])),
                    ),
                    ("model_group".to_owned(), Json::Text("group-1".to_owned())),
                ])),
            );
            let payload = built(configuration, &chat_response());
            assert_eq!(payload.model_id.as_deref(), Some("model-id-1"));
            assert_eq!(payload.model_group.as_deref(), Some("group-1"));
        }
        ContractBehavior::ResponseCostFields => {
            let cost: Json = serde_json::from_str("0.5").unwrap();
            let mut state = state(None);
            state
                .model_call_details
                .insert("response_cost".to_owned(), cost);
            let payload = built_with(PayloadSources::default(), &state, &chat_response());
            let expected: Json = serde_json::from_str("0.5").unwrap();
            assert_eq!(payload.response_cost.to_string(), "0.5");
            assert_eq!(payload.hidden_params.response_cost, Some(expected));
        }
        ContractBehavior::StoredOverheadWins => {
            let response: Json = serde_json::from_str(
                r#"{"id": "chatcmpl-1", "model": "gpt-4o", "choices": [],
                    "_hidden_params": {"litellm_overhead_time_ms": 3.0}}"#,
            )
            .unwrap();
            let mut state = state(None);
            state
                .response_timing_metrics
                .insert("litellm_overhead_time_ms".to_owned(), 12.5_f64);
            let payload = built_with(PayloadSources::default(), &state, &response);
            assert_eq!(
                payload
                    .hidden_params
                    .litellm_overhead_time_ms
                    .map(|value| value.0.to_string()),
                Some("3.0".to_owned())
            );
        }
        ContractBehavior::UnbilledManagementCalls => {
            let mut plain = init(None);
            plain.call_type = "vector_store_create".to_owned();
            let payload = built(plain, &chat_response());
            assert_eq!(
                (
                    payload.prompt_tokens,
                    payload.completion_tokens,
                    payload.total_tokens
                ),
                (0, 0, 0)
            );
            let billed = built(
                management_state("background_response_cost_poll"),
                &chat_response(),
            );
            assert_eq!(
                (
                    billed.prompt_tokens,
                    billed.completion_tokens,
                    billed.total_tokens
                ),
                (2, 3, 5)
            );
            let other = built(management_state("other_call"), &chat_response());
            assert_eq!(
                (
                    other.prompt_tokens,
                    other.completion_tokens,
                    other.total_tokens
                ),
                (0, 0, 0)
            );
        }
        ContractBehavior::ModelParameters => {
            let mut state = state(None);
            state.optional_params.insert(
                "temperature".to_owned(),
                serde_json::from_str("0.5").unwrap(),
            );
            let payload = built_with(PayloadSources::default(), &state, &chat_response());
            let expected: Json = serde_json::from_str("0.5").unwrap();
            assert_eq!(payload.model_parameters.get("temperature"), Some(&expected));
        }
        ContractBehavior::PassThroughFields => {
            let response: Json = serde_json::from_str(
                r#"{"id": "chatcmpl-1", "model": "gpt-4o", "choices": [],
                    "_hidden_params": {"cache_key": "cache-1"}}"#,
            )
            .unwrap();
            let mut state = state(None);
            state.model_call_details.insert(
                "response_cost_failure_debug_information".to_owned(),
                Json::Text("cost-debug".to_owned()),
            );
            state.model_call_details.insert(
                "zero_cost_diagnostic".to_owned(),
                Json::Text("zero-debug".to_owned()),
            );
            state.litellm_params.insert(
                "api_base".to_owned(),
                Json::Text("https://api.openai.com/".to_owned()),
            );
            let payload = built_with(PayloadSources::default(), &state, &response);
            assert_eq!(
                payload.cost_failure_info,
                Some(Json::Text("cost-debug".to_owned()))
            );
            assert_eq!(
                payload.zero_cost_diagnostic,
                Some(Json::Text("zero-debug".to_owned()))
            );
            assert_eq!(payload.api_base, "https://api.openai.com");
            assert_eq!(payload.cache_key.as_deref(), Some("cache-1"));
            assert_eq!(payload.hidden_params.cache_key.as_deref(), Some("cache-1"));
        }
        ContractBehavior::ServedTexts => {
            let mut state = state(None);
            record_served_output_texts(&mut state, Some(vec![Some("served text".to_owned())]));
            let payload = built_with(PayloadSources::default(), &state, &chat_response());
            let response = payload.response.expect("the response is logged");
            let Some(Json::List(choices)) = object_field(&response, "choices") else {
                panic!("choices stay a list");
            };
            let message = object_field(&choices[0], "message").expect("a choice keeps its message");
            assert_eq!(
                object_field(message, "content"),
                Some(&Json::Text("served text".to_owned()))
            );
        }
        ContractBehavior::RequestTags => {
            let mut configuration = init(None);
            configuration.litellm_params.insert(
                "proxy_server_request".to_owned(),
                serde_json::from_str(r#"{"headers": {"user-agent": "my-client/1.0"}}"#).unwrap(),
            );
            let payload = built(configuration, &chat_response());
            assert_eq!(
                payload.request_tags,
                vec![
                    Json::Text("User-Agent: my-client".to_owned()),
                    Json::Text("User-Agent: my-client/1.0".to_owned()),
                ]
            );
        }
        ContractBehavior::ObjectOrEmpty => {
            let object: Json = serde_json::from_str(r#"{"id": "one"}"#).unwrap();
            assert_eq!(
                object_or_empty(Some(&object)),
                IndexMap::from([("id".to_owned(), Json::Text("one".to_owned()))])
            );
            assert!(object_or_empty(None).is_empty());
            let text = Json::Text("not an object".to_owned());
            assert!(object_or_empty(Some(&text)).is_empty());
        }
        ContractBehavior::StdoutEmitterOutput => {
            let payload = payload_from(BASE_PAYLOAD);
            let file = tempfile::tempfile().expect("a scratch file");
            let saved = dup(stdout()).expect("stdout can be duplicated");
            dup2_stdout(file.as_fd()).expect("stdout can be redirected");
            let emitter = StdoutEmitter {
                settings: EmitSettings::from_env(Some("1")),
            };
            let emitted = emitter.emit(&payload);
            dup2_stdout(saved).expect("stdout can be restored");
            assert!(emitted.is_ok(), "the stdout emitter writes the payload");
            let mut reader = BufReader::new(&file);
            reader.seek(SeekFrom::Start(0)).expect("the file rewinds");
            let mut printed_bytes = Vec::new();
            reader
                .read_to_end(&mut printed_bytes)
                .expect("the printed line reads back");
            let line = printed_bytes
                .strip_suffix(b"\n")
                .expect("the printed line ends with a newline");
            let printed: Json = serde_json::from_slice(line).expect("the line is JSON");
            assert_eq!(
                serde_json::to_string(&printed).unwrap(),
                serde_json::to_string(&payload).unwrap()
            );
        }
        ContractBehavior::NonStreamTiming => {
            let mut state = state(None);
            state.completion_start_time = Some(super::time("2024-01-01T00:00:00.5+00:00"));
            let payload = built_with(PayloadSources::default(), &state, &chat_response());
            assert_eq!(payload.completion_start_time.to_string(), "1704067200.5");
            assert_eq!(payload.end_time.to_string(), "1704067201.0");
            assert_eq!(payload.response_time.to_string(), "1.0");
        }
        ContractBehavior::NullRoutingDecision => {
            let mut configuration = init(None);
            configuration.litellm_params.insert(
                "metadata".to_owned(),
                Json::Object(IndexMap::from([(
                    "routing_decision".to_owned(),
                    Json::Null,
                )])),
            );
            let payload = built(configuration, &chat_response());
            assert!(payload.autorouter_savings_estimate.is_none());
        }
        ContractBehavior::ResponseIdFallback => {
            let response: Json =
                serde_json::from_str(r#"{"model": "gpt-4o", "choices": []}"#).unwrap();
            let payload = built(init(None), &response);
            assert_eq!(payload.id, "call-1");
            let logged = payload.response.expect("the response is logged");
            assert!(object_field(&logged, "id").is_none());
        }
    }
}

fn management_state(origin: &str) -> LoggingInit {
    let mut configuration = init(None);
    configuration.call_type = "vector_store_create".to_owned();
    configuration.litellm_params.insert(
        "metadata".to_owned(),
        Json::Object(IndexMap::from([(
            "internal_call_origin".to_owned(),
            Json::Text(origin.to_owned()),
        )])),
    );
    configuration
}
