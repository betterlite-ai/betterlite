use std::sync::Arc;

use betterlite_config::sections::logging_privacy::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::{
    builder, builder_with, chat_response, end, init, object_field, sources_with, start, state,
    state_from,
};
use crate::exporter_options::redaction::REDACTED_TEXT;
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::status::GuardrailStatus;
use crate::logging_failure::{FailurePayloadInput, FailurePayloadSource};
use crate::logging_metadata::exception::ExceptionFacts;
use crate::logging_state::LoggingInit;
use crate::logging_state::types::Messages;
use crate::logging_success::ports::StandardPayloadBuilder;
use crate::payload::deps::{
    AbsentAutorouterAudit, AbsentSavedCacheCost, AutorouterAudit, PayloadClock, PayloadSettings,
    PayloadSources, SavedCacheCost, SavingsRequest, SystemPayloadClock, TagConfig,
};

fn built(
    sources: PayloadSources,
    configuration: LoggingInit,
    response: &Json,
) -> StandardLoggingPayload {
    let state = state_from(configuration);
    StandardPayloadBuilder::build(
        &builder_with(sources, PayloadSettings::default(), Settings::default()),
        &state,
        response,
        start(),
        end(),
    )
    .expect("the payload builds")
}

fn built_failure(
    sources: PayloadSources,
    settings: PayloadSettings,
    configuration: LoggingInit,
    exception: &ExceptionFacts,
) -> StandardLoggingPayload {
    let state = state_from(configuration);
    FailurePayloadSource::build(
        &builder_with(sources, settings, Settings::default()),
        FailurePayloadInput {
            state: &state,
            exception,
            start_time: start(),
            end_time: end(),
            error_str: "the request failed",
        },
    )
    .expect("the failure payload builds")
}

fn set_params(configuration: &mut LoggingInit, key: &str, value: Json) {
    configuration.litellm_params.insert(key.to_owned(), value);
}

fn estimate_field(payload: &StandardLoggingPayload, key: &str) -> Option<Json> {
    payload
        .autorouter_savings_estimate
        .as_ref()
        .and_then(|estimate| estimate.get(key))
        .cloned()
}

pub(super) struct FixedClock(pub(super) f64);

impl PayloadClock for FixedClock {
    fn now_epoch_seconds(&self) -> f64 {
        self.0
    }
}

struct FixedSavedCost(Decimal);

impl SavedCacheCost for FixedSavedCost {
    fn saved_cache_cost(&self, _init_response_obj: &Json) -> Option<Decimal> {
        Some(self.0)
    }
}

struct FixedAudit {
    savings: Option<Decimal>,
    observation: Option<String>,
    active: bool,
}

impl AutorouterAudit for FixedAudit {
    fn savings(&self, _request: &SavingsRequest<'_>) -> Option<Decimal> {
        self.savings
    }

    fn baseline_observation(&self) -> Option<String> {
        self.observation.clone()
    }

    fn baseline_context_active(&self) -> bool {
        self.active
    }
}

#[derive(Clone, Copy, Debug)]
enum BuilderBehavior {
    BuiltPayloadPrivacyMessages,
    AzureRouterStamp,
    AzureRouterDeployment,
    GuardrailInformation,
    CacheHitSavedCost,
    CombinedUsage,
    AutorouterEstimateShapes,
    AutorouterSuppression,
    ExceptionHeaders,
    EndUserFallback,
    OverheadTiming,
    NonTextSystemIgnored,
}

#[rstest]
#[case::built_payload_keeps_the_system_prompt(BuilderBehavior::BuiltPayloadPrivacyMessages)]
#[case::azure_router_stamp_names_the_model(BuilderBehavior::AzureRouterStamp)]
#[case::azure_router_deployment_names_the_model(BuilderBehavior::AzureRouterDeployment)]
#[case::guardrail_cost_and_status_reach_the_payload(BuilderBehavior::GuardrailInformation)]
#[case::cache_hit_suffixes_the_id_and_saves_cost(BuilderBehavior::CacheHitSavedCost)]
#[case::combined_usage_wins_over_the_response(BuilderBehavior::CombinedUsage)]
#[case::autorouter_estimates_follow_the_source(BuilderBehavior::AutorouterEstimateShapes)]
#[case::autorouter_savings_stay_off(BuilderBehavior::AutorouterSuppression)]
#[case::exception_headers_and_tracebacks_follow_the_source(BuilderBehavior::ExceptionHeaders)]
#[case::end_user_falls_back_to_the_proxy_body(BuilderBehavior::EndUserFallback)]
#[case::overhead_timing_fills_only_on_success(BuilderBehavior::OverheadTiming)]
#[case::non_text_system_is_ignored(BuilderBehavior::NonTextSystemIgnored)]
#[tokio::test]
async fn logging_payloads_logging_payload_tests_behavior(#[case] behavior: BuilderBehavior) {
    match behavior {
        BuilderBehavior::BuiltPayloadPrivacyMessages => {
            let mut configuration = init(None);
            configuration.messages = Some(Messages::Values(vec![Json::Object(IndexMap::from([
                ("role".to_owned(), Json::Text("user".to_owned())),
                ("content".to_owned(), Json::Text("hello".to_owned())),
            ]))]));
            let mut state = state_from(configuration);
            state.model_call_details.insert(
                "system".to_owned(),
                Json::Text("log the system prompt".to_owned()),
            );
            let privacy = Settings {
                turn_off_message_logging: true,
                redact_user_api_key_info: false,
            };
            let payload = StandardPayloadBuilder::build(
                &builder_with(
                    PayloadSources::default(),
                    PayloadSettings::default(),
                    privacy,
                ),
                &state,
                &chat_response(),
                start(),
                end(),
            )
            .expect("the payload builds");
            let Json::List(messages) = payload.messages.unwrap() else {
                panic!("messages stay a list");
            };
            assert_eq!(messages.len(), 2);
            assert_eq!(
                object_field(&messages[0], "content"),
                Some(&Json::Text("log the system prompt".to_owned()))
            );
            assert_eq!(
                object_field(&messages[1], "content"),
                Some(&Json::Text(REDACTED_TEXT.to_owned()))
            );
        }
        BuilderBehavior::AzureRouterStamp => {
            let response = serde_json::from_str(
                r#"{
                    "id": "chatcmpl-router",
                    "model": "gpt-4o",
                    "choices": [{"index": 0, "message": {"role": "assistant", "content": "hello"}}],
                    "_hidden_params": {"azure_model_router_selected_model": "router-picked"}
                }"#,
            )
            .unwrap();
            let payload = built(PayloadSources::default(), init(None), &response);
            assert_eq!(payload.model, "router-picked");
        }
        BuilderBehavior::AzureRouterDeployment => {
            let response = serde_json::from_str(
                r#"{
                    "id": "chatcmpl-router",
                    "model": "served-by-router",
                    "choices": [{"index": 0, "message": {"role": "assistant", "content": "hello"}}],
                    "_hidden_params": {"litellm_model_name": "model_router/gpt-4o"}
                }"#,
            )
            .unwrap();
            let payload = built(PayloadSources::default(), init(None), &response);
            assert_eq!(payload.model, "served-by-router");
        }
        BuilderBehavior::GuardrailInformation => {
            let mut configuration = init(None);
            set_params(
                &mut configuration,
                "metadata",
                serde_json::from_str(
                    r#"{
                        "standard_logging_guardrail_information": [
                            {"guardrail_name": "pii", "guardrail_status": "guardrail_intervened",
                             "guardrail_cost": 0.25, "guardrail_cost_in_spend": true}
                        ]
                    }"#,
                )
                .unwrap(),
            );
            let payload = built(PayloadSources::default(), configuration, &chat_response());
            assert_eq!(payload.response_cost.to_string(), "0.25");
            let breakdown = payload
                .cost_breakdown
                .expect("the guardrail cost makes a breakdown");
            assert_eq!(
                breakdown.guardrail_cost.map(|cost| cost.to_string()),
                Some("0.25".to_owned())
            );
            assert_eq!(
                payload.status_fields.guardrail_status,
                GuardrailStatus::GuardrailIntervened
            );
            let guardrails = payload
                .guardrail_information
                .expect("the guardrails are logged");
            assert_eq!(guardrails.len(), 1);
            assert_eq!(
                guardrails[0].guardrail_name.clone().flatten().as_deref(),
                Some("pii")
            );
        }
        BuilderBehavior::CacheHitSavedCost => {
            let mut state = state(None);
            state
                .model_call_details
                .insert("cache_hit".to_owned(), Json::Bool(true));
            let sources = sources_with(
                Arc::new(FixedClock(1234.0)),
                Arc::new(FixedSavedCost(Decimal::new(25, 2))),
                Arc::new(AbsentAutorouterAudit),
            );
            let payload = StandardPayloadBuilder::build(
                &builder_with(sources, PayloadSettings::default(), Settings::default()),
                &state,
                &chat_response(),
                start(),
                end(),
            )
            .expect("the payload builds");
            assert_eq!(payload.id, "chatcmpl-1_cache_hit1234.0");
            assert_eq!(payload.cache_hit, Some(true));
            assert_eq!(payload.saved_cache_cost.to_string(), "0.25");
            let response = payload.response.expect("the response is logged");
            assert_eq!(
                object_field(&response, "id"),
                Some(&Json::Text("chatcmpl-1".to_owned()))
            );
        }
        BuilderBehavior::CombinedUsage => {
            let response_usage = StandardPayloadBuilder::build(
                &builder(Settings::default()),
                &state(None),
                &chat_response(),
                start(),
                end(),
            )
            .expect("the payload builds");
            assert_eq!(
                (
                    response_usage.prompt_tokens,
                    response_usage.completion_tokens,
                    response_usage.total_tokens
                ),
                (2, 3, 5)
            );
            let mut state = state(None);
            state.model_call_details.insert(
                "combined_usage_object".to_owned(),
                serde_json::from_str(
                    r#"{"prompt_tokens": 7, "completion_tokens": 11, "total_tokens": 18}"#,
                )
                .unwrap(),
            );
            let payload = StandardPayloadBuilder::build(
                &builder(Settings::default()),
                &state,
                &chat_response(),
                start(),
                end(),
            )
            .expect("the payload builds");
            assert_eq!(
                (
                    payload.prompt_tokens,
                    payload.completion_tokens,
                    payload.total_tokens
                ),
                (7, 11, 18)
            );
            let usage = payload.metadata.usage_object.expect("the usage is logged");
            assert_eq!(
                object_field(&usage, "total_tokens"),
                Some(&Json::Number(Number::from(18_i64)))
            );
        }
        BuilderBehavior::AutorouterEstimateShapes => {
            let mut configuration = init(None);
            set_params(
                &mut configuration,
                "metadata",
                serde_json::from_str(r#"{"routing_decision": {"selected_model": "gpt-4o"}}"#)
                    .unwrap(),
            );
            let estimated_sources = sources_with(
                Arc::new(SystemPayloadClock),
                Arc::new(AbsentSavedCacheCost),
                Arc::new(FixedAudit {
                    savings: Some(Decimal::new(5, 2)),
                    observation: None,
                    active: false,
                }),
            );
            let estimated = built(estimated_sources, configuration.clone(), &chat_response());
            assert_eq!(
                estimated
                    .autorouter_savings
                    .map(|value| value.to_string())
                    .as_deref(),
                Some("0.05")
            );
            assert_eq!(
                estimated
                    .autorouter_savings_estimate
                    .as_ref()
                    .and_then(|estimate| estimate.get("version")),
                Some(&Json::Number(Number::from(1_i64)))
            );
            assert_eq!(
                estimate_field(&estimated, "status"),
                Some(Json::Text("estimated".to_owned()))
            );
            let unknown_sources = sources_with(
                Arc::new(SystemPayloadClock),
                Arc::new(AbsentSavedCacheCost),
                Arc::new(FixedAudit {
                    savings: None,
                    observation: None,
                    active: false,
                }),
            );
            let unknown = built(unknown_sources, configuration.clone(), &chat_response());
            assert!(unknown.autorouter_savings.is_none());
            assert_eq!(
                estimate_field(&unknown, "reason"),
                Some(Json::Text("baseline_unavailable".to_owned()))
            );
            let pending_sources = sources_with(
                Arc::new(SystemPayloadClock),
                Arc::new(AbsentSavedCacheCost),
                Arc::new(FixedAudit {
                    savings: Some(Decimal::new(5, 2)),
                    observation: Some("baseline-1".to_owned()),
                    active: false,
                }),
            );
            let pending = built(pending_sources, configuration, &chat_response());
            assert_eq!(
                estimate_field(&pending, "version"),
                Some(Json::Number(Number::from(3_i64)))
            );
            assert_eq!(
                estimate_field(&pending, "reason"),
                Some(Json::Text("pending_projection".to_owned()))
            );
            assert_eq!(
                estimate_field(&pending, "status"),
                Some(Json::Text("unknown".to_owned()))
            );
        }
        BuilderBehavior::AutorouterSuppression => {
            let mut configuration = init(None);
            set_params(
                &mut configuration,
                "metadata",
                serde_json::from_str(r#"{"routing_decision": {"selected_model": "gpt-4o"}}"#)
                    .unwrap(),
            );
            let sources = sources_with(
                Arc::new(SystemPayloadClock),
                Arc::new(AbsentSavedCacheCost),
                Arc::new(FixedAudit {
                    savings: Some(Decimal::new(5, 2)),
                    observation: None,
                    active: false,
                }),
            );
            let mut cached_state = state_from(configuration.clone());
            cached_state
                .model_call_details
                .insert("cache_hit".to_owned(), Json::Bool(true));
            let cached = StandardPayloadBuilder::build(
                &builder_with(sources, PayloadSettings::default(), Settings::default()),
                &cached_state,
                &chat_response(),
                start(),
                end(),
            )
            .expect("the payload builds");
            assert!(cached.autorouter_savings.is_none());
            assert_eq!(
                estimate_field(&cached, "reason"),
                Some(Json::Text("baseline_unavailable".to_owned()))
            );
            let active_sources = sources_with(
                Arc::new(SystemPayloadClock),
                Arc::new(AbsentSavedCacheCost),
                Arc::new(FixedAudit {
                    savings: Some(Decimal::new(5, 2)),
                    observation: None,
                    active: true,
                }),
            );
            let active = built(active_sources, configuration.clone(), &chat_response());
            assert!(active.autorouter_savings.is_none());
            let exception: ExceptionFacts = serde_json::from_str(r#"{"message": "nope"}"#).unwrap();
            let failed = built_failure(
                PayloadSources::default(),
                PayloadSettings::default(),
                configuration,
                &exception,
            );
            assert!(failed.autorouter_savings.is_none());
        }
        BuilderBehavior::ExceptionHeaders => {
            let exception: ExceptionFacts = serde_json::from_str(
                r#"{
                    "class_name": "InvalidRequestError",
                    "message": "bad request",
                    "code": "400",
                    "llm_provider": "openai",
                    "headers": {"x-request-id": "from-exception"},
                    "response_headers": {"x-request-id": "from-response", "x-other": "other"},
                    "litellm_response_headers": {"x-request-id": "from-litellm"},
                    "traceback_lines": ["line one\n", "line two\n"]
                }"#,
            )
            .unwrap();
            let exception_payload = built_failure(
                PayloadSources::default(),
                PayloadSettings::default(),
                init(None),
                &exception,
            );
            let headers = exception_payload
                .hidden_params
                .additional_headers
                .expect("the exception headers are logged");
            assert_eq!(
                headers.get("x-request-id"),
                Some(&Json::Text("from-exception".to_owned()))
            );
            assert!(!headers.contains_key("x_other"));
            let information = exception_payload
                .error_information
                .expect("the error information is logged");
            assert_eq!(
                information
                    .error_provider_request_id
                    .clone()
                    .flatten()
                    .as_deref(),
                Some("from-exception")
            );
            let from_response: ExceptionFacts = serde_json::from_str(
                r#"{
                    "message": "bad request",
                    "response_headers": {"x-request-id": "from-response"},
                    "litellm_response_headers": {"x-request-id": "from-litellm"}
                }"#,
            )
            .unwrap();
            let response_payload = built_failure(
                PayloadSources::default(),
                PayloadSettings::default(),
                init(None),
                &from_response,
            );
            assert_eq!(
                response_payload
                    .hidden_params
                    .additional_headers
                    .expect("the response headers are logged")
                    .get("x-request-id"),
                Some(&Json::Text("from-response".to_owned()))
            );
            let from_litellm: ExceptionFacts = serde_json::from_str(
                r#"{
                    "message": "bad request",
                    "litellm_response_headers": {"x-request-id": "from-litellm"}
                }"#,
            )
            .unwrap();
            let litellm_payload = built_failure(
                PayloadSources::default(),
                PayloadSettings::default(),
                init(None),
                &from_litellm,
            );
            assert_eq!(
                litellm_payload
                    .hidden_params
                    .additional_headers
                    .expect("the litellm headers are logged")
                    .get("x-request-id"),
                Some(&Json::Text("from-litellm".to_owned()))
            );
            assert_eq!(
                information.traceback.flatten().as_deref(),
                Some("line one\nline two\n")
            );
            let proxy: ExceptionFacts = serde_json::from_str(
                r#"{
                    "class_name": "ProxyException",
                    "message": "missing key",
                    "code": "401",
                    "traceback_lines": ["proxy line\n"]
                }"#,
            )
            .unwrap();
            let quiet = built_failure(
                PayloadSources::default(),
                PayloadSettings::default(),
                init(None),
                &proxy,
            );
            assert_eq!(
                quiet
                    .error_information
                    .unwrap()
                    .traceback
                    .flatten()
                    .as_deref(),
                Some("")
            );
            let loud = built_failure(
                PayloadSources::default(),
                PayloadSettings {
                    tag_config: TagConfig::default(),
                    request_correlation_in_logs: false,
                    log_client_error_tracebacks: true,
                },
                init(None),
                &proxy,
            );
            assert_eq!(
                loud.error_information
                    .unwrap()
                    .traceback
                    .flatten()
                    .as_deref(),
                Some("proxy line\n")
            );
        }
        BuilderBehavior::EndUserFallback => {
            let mut configuration = init(None);
            set_params(
                &mut configuration,
                "proxy_server_request",
                serde_json::from_str(r#"{"body": {"user": "end-user-1"}}"#).unwrap(),
            );
            let payload = built(PayloadSources::default(), configuration, &chat_response());
            assert_eq!(payload.end_user.as_deref(), Some("end-user-1"));
        }
        BuilderBehavior::OverheadTiming => {
            let mut state = state(None);
            state
                .response_timing_metrics
                .insert("litellm_overhead_time_ms".to_owned(), 12.5_f64);
            let payload = StandardPayloadBuilder::build(
                &builder(Settings::default()),
                &state,
                &chat_response(),
                start(),
                end(),
            )
            .expect("the payload builds");
            assert_eq!(
                payload
                    .hidden_params
                    .litellm_overhead_time_ms
                    .map(|value| value.0.to_string()),
                Some("12.5".to_owned())
            );
            let exception: ExceptionFacts = serde_json::from_str(r#"{"message": "nope"}"#).unwrap();
            let failed = FailurePayloadSource::build(
                &builder_with(
                    PayloadSources::default(),
                    PayloadSettings::default(),
                    Settings::default(),
                ),
                FailurePayloadInput {
                    state: &state,
                    exception: &exception,
                    start_time: start(),
                    end_time: end(),
                    error_str: "the request failed",
                },
            )
            .expect("the failure payload builds");
            assert!(failed.hidden_params.litellm_overhead_time_ms.is_none());
        }
        BuilderBehavior::NonTextSystemIgnored => {
            let mut configuration = init(None);
            configuration.messages = Some(Messages::Values(vec![Json::Object(IndexMap::from([
                ("role".to_owned(), Json::Text("user".to_owned())),
                ("content".to_owned(), Json::Text("hello".to_owned())),
            ]))]));
            let mut state = state_from(configuration);
            state.model_call_details.insert(
                "system".to_owned(),
                serde_json::from_str(r#"[{"type": "text", "text": "block system prompt"}]"#)
                    .unwrap(),
            );
            let payload = StandardPayloadBuilder::build(
                &builder(Settings::default()),
                &state,
                &chat_response(),
                start(),
                end(),
            )
            .expect("a block-list system does not fail the payload");
            let Json::List(messages) = payload.messages.expect("the messages are logged") else {
                panic!("messages stay a list");
            };
            assert_eq!(messages.len(), 1);
            assert_eq!(
                object_field(&messages[0], "role"),
                Some(&Json::Text("user".to_owned()))
            );
            assert_eq!(
                object_field(&messages[0], "content"),
                Some(&Json::Text("hello".to_owned()))
            );
        }
    }
}
