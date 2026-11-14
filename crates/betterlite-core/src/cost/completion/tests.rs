#![cfg(test)]

use betterlite_config::cost_adjustments::{AdjustmentEntry, MarginFieldEntry};
use betterlite_models::decisions::Json;
use betterlite_models::ocr_pricing::{OcrBilling, OcrResponseKind, OcrUsage};
use betterlite_models::protocol_cost::{A2aCall, A2aUsage, ProtocolCall};
use betterlite_models::tool_signals::response::{
    Annotation, ChatChoice, ChatMessage, ChatView, ToolBillingResponse,
};
use betterlite_models::usage_sum::Usage as SumUsage;
use indexmap::IndexMap;
use rstest::rstest;
use std::time::{Duration, UNIX_EPOCH};

use self::rig::{
    Recorded, Refusing, Stored, args, chat_usage, chat_view, dec, map, number, rig, searching,
};
use super::error::{ChargeInputError, CompletionCostError};
use super::reading;
use crate::cost::prepare::ResponseView;
use crate::cost::prepare::error::PrepareError;
use crate::cost::realtime::RealtimeBilling;
use crate::routes::CallType;

mod lookup;
pub(crate) mod rig;

const GPT: &[(&str, &str)] = &[
    ("input_cost_per_token", "0.001"),
    ("output_cost_per_token", "0.002"),
];

fn zero_adjustments(recorded: &Recorded) {
    assert_eq!(recorded.discount_percent, Some(dec("0")));
    assert_eq!(recorded.discount_amount, Some(dec("0")));
    assert_eq!(recorded.margin_percent, Some(dec("0")));
    assert_eq!(recorded.margin_fixed_amount, Some(dec("0")));
    assert_eq!(recorded.margin_total_amount, Some(dec("0")));
}

#[tokio::test]
async fn a_chat_charge_prices_both_token_counts_and_records_the_breakdown() {
    let usage = chat_usage(10, 5);
    let view = chat_view(&usage, Some("gpt-4o"), None);
    let mut stored = Stored::default();
    let charge = rig(&[("gpt-4o", GPT)])
        .charge(&args(Some(&view)), Some(&mut stored))
        .await
        .unwrap();
    assert_eq!(charge, dec("0.02"));
    let recorded = stored.0.expect("the breakdown was stored");
    assert_eq!(recorded.input_cost, dec("0.01"));
    assert_eq!(recorded.output_cost, dec("0.01"));
    assert_eq!(recorded.total_cost, dec("0.02"));
    assert_eq!(recorded.original_cost, Some(dec("0.02")));
    assert_eq!(recorded.tool_usage_cost, dec("0"));
    assert!(recorded.token_costs.iter().all(Option::is_some));
    zero_adjustments(&recorded);
}

#[tokio::test]
async fn a_failed_candidate_hands_over_to_the_next_one() {
    let usage = chat_usage(10, 5);
    let view = chat_view(&usage, Some("gpt-4o"), None);
    let mut call = args(Some(&view));
    call.billing.model = Some("mystery");
    let charge = rig(&[("gpt-4o", GPT)])
        .charge(&call, None::<&mut Stored>)
        .await;
    assert_eq!(charge.unwrap(), dec("0.02"));
}

#[tokio::test]
async fn the_last_candidates_error_is_the_one_raised() {
    let usage = chat_usage(10, 5);
    let view = chat_view(&usage, Some("unknown"), None);
    let mut call = args(Some(&view));
    call.billing.model = None;
    let error = rig(&[("gpt-4o", GPT)])
        .charge(&call, None::<&mut Stored>)
        .await
        .unwrap_err();
    assert!(matches!(error, CompletionCostError::Price(_)), "{error}");
    let mut unnamed = args(None);
    unnamed.billing.model = None;
    let nothing = rig(&[]).charge(&unnamed, None::<&mut Stored>).await;
    assert!(matches!(
        nothing,
        Err(CompletionCostError::Prepare(PrepareError::ModelMissing))
    ));
}

#[tokio::test]
async fn a_protocol_charge_returns_before_the_token_stage() {
    let call = ProtocolCall::A2a(A2aCall {
        response_cost: number("0.25"),
        cost_per_query: Json::Null,
        input_cost_per_token: Json::Null,
        output_cost_per_token: Json::Null,
        usage: A2aUsage::Absent,
    });
    let mut request = args(None);
    request.billing.call_type = Some(CallType::SendMessage);
    request.charge.response.protocol = Some(&call);
    let mut stored = Stored::default();
    let charge = rig(&[]).charge(&request, Some(&mut stored)).await.unwrap();
    assert_eq!(charge, dec("0.25"));
    assert!(stored.0.is_none());
}

#[tokio::test]
async fn the_discount_applies_before_the_margin_and_both_are_recorded() {
    let usage = chat_usage(10, 5);
    let view = chat_view(&usage, Some("gpt-4o"), None);
    let mut priced = rig(&[("gpt-4o", GPT)]);
    priced
        .discount
        .insert("openai".to_owned(), AdjustmentEntry::Percent(dec("0.5")));
    priced.margin.insert(
        "openai".to_owned(),
        AdjustmentEntry::Split {
            percentage: MarginFieldEntry::Number(dec("0.1")),
            fixed_amount: MarginFieldEntry::Number(dec("0.001")),
        },
    );
    let mut stored = Stored::default();
    let charge = priced
        .charge(&args(Some(&view)), Some(&mut stored))
        .await
        .unwrap();
    assert_eq!(charge, dec("0.012"));
    let recorded = stored.0.unwrap();
    assert_eq!(recorded.original_cost, Some(dec("0.02")));
    assert_eq!(recorded.discount_percent, Some(dec("0.5")));
    assert_eq!(recorded.discount_amount, Some(dec("0.01")));
    assert_eq!(recorded.margin_percent, Some(dec("0.1")));
    assert_eq!(recorded.margin_fixed_amount, Some(dec("0.001")));
    assert_eq!(recorded.margin_total_amount, Some(dec("0.002")));
    assert_eq!(recorded.total_cost, dec("0.012"));
}

#[tokio::test]
async fn an_empty_margin_map_leaves_the_discounted_cost_alone() {
    let usage = chat_usage(10, 5);
    let view = chat_view(&usage, Some("gpt-4o"), None);
    let mut priced = rig(&[("gpt-4o", GPT)]);
    priced
        .discount
        .insert("openai".to_owned(), AdjustmentEntry::Percent(dec("0.5")));
    let mut stored = Stored::default();
    let charge = priced
        .charge(&args(Some(&view)), Some(&mut stored))
        .await
        .unwrap();
    assert_eq!(charge, dec("0.01"));
    let recorded = stored.0.unwrap();
    assert_eq!(recorded.margin_percent, Some(dec("0")));
    assert_eq!(recorded.margin_total_amount, Some(dec("0")));
    assert_eq!(recorded.discount_amount, Some(dec("0.01")));
}

#[tokio::test]
async fn built_in_tool_fees_join_the_total_before_the_discount() {
    let usage = chat_usage(10, 5);
    let view = chat_view(&usage, Some("gpt-4o"), None);
    let mut call = args(Some(&view));
    call.charge.tools.response = ToolBillingResponse::Chat(ChatView {
        choices: vec![ChatChoice {
            message: Some(ChatMessage {
                annotations: Some(vec![Annotation {
                    annotation_type: Some(Json::Text("url_citation".to_owned())),
                }]),
            }),
        }],
        usage: None,
    });
    let mut priced = rig(&[("gpt-4o", GPT)]).with_tool_prices(searching());
    priced
        .discount
        .insert("openai".to_owned(), AdjustmentEntry::Percent(dec("0.5")));
    let mut stored = Stored::default();
    let charge = priced.charge(&call, Some(&mut stored)).await.unwrap();
    assert_eq!(charge, dec("0.015"));
    let recorded = stored.0.unwrap();
    assert_eq!(recorded.tool_usage_cost, dec("0.01"));
    assert_eq!(recorded.original_cost, Some(dec("0.03")));
}

const ROUTED: &[(&str, &str)] = &[("input_cost_per_token", "0.0001")];

#[rstest]
#[case::router_named_hidden_model("azure_ai", r#"{"model":"azure_ai/model-router-x"}"#, "0.021")]
#[case::the_other_hidden_name(
    "azure_ai",
    r#"{"litellm_model_name":"azure_ai/model-router-x"}"#,
    "0.021"
)]
#[case::plain_hidden_model("azure_ai", r#"{"model":"gpt-4o"}"#, "0.02")]
#[case::empty_hidden_model("azure_ai", r#"{"model":""}"#, "0.02")]
#[case::empty_hidden_model_falls_back_to_the_other_name(
    "azure_ai",
    r#"{"model":"","litellm_model_name":"azure_ai/model-router-x"}"#,
    "0.021"
)]
#[case::another_provider("openai", r#"{"model":"azure_ai/model-router-x"}"#, "0.02")]
#[tokio::test]
async fn the_router_fee_needs_azure_ai_and_a_router_shaped_hidden_model(
    #[case] provider: &str,
    #[case] hidden: &str,
    #[case] expected: &str,
) {
    let usage = chat_usage(10, 5);
    let hidden = map(hidden);
    let view = chat_view(&usage, Some("gpt-4o"), Some(&hidden));
    let mut call = args(Some(&view));
    call.billing.custom_llm_provider = Some(provider);
    let mut stored = Stored::default();
    let charge = rig(&[("gpt-4o", GPT), ("model_router", ROUTED)])
        .charge(&call, Some(&mut stored))
        .await
        .unwrap();
    assert_eq!(charge, dec(expected));
    let fees = stored.0.unwrap().additional_costs;
    assert_eq!(fees.is_some(), expected == "0.021");
}

#[rstest]
#[case::router_named_logging_model("azure_ai/model-router-x", "{}", "0.021")]
#[case::router_named_hidden_model_beats_a_plain_logging_one(
    "gpt-4o",
    r#"{"model":"azure_ai/model-router-x"}"#,
    "0.021"
)]
#[case::router_named_logging_model_beats_a_plain_hidden_one(
    "azure_ai/model-router-x",
    r#"{"model":"gpt-4o"}"#,
    "0.021"
)]
#[case::plain_logging_and_hidden_models("gpt-4o", r#"{"model":"gpt-4o"}"#, "0.02")]
#[tokio::test]
async fn the_logging_model_names_the_fee_unless_a_router_hidden_model_replaces_it(
    #[case] logging: &str,
    #[case] hidden: &str,
    #[case] expected: &str,
) {
    let usage = chat_usage(10, 5);
    let hidden = map(hidden);
    let view = chat_view(&usage, Some("gpt-4o"), Some(&hidden));
    let mut call = args(Some(&view));
    call.billing.custom_llm_provider = Some("azure_ai");
    call.charge.logging.model = Some(logging);
    let charge = rig(&[("gpt-4o", GPT), ("model_router", ROUTED)])
        .charge(&call, None::<&mut Stored>)
        .await
        .unwrap();
    assert_eq!(charge, dec(expected));
}

#[tokio::test]
async fn a_router_shaped_priced_model_owes_no_extra_fee() {
    let usage = chat_usage(10, 5);
    let view = chat_view(&usage, Some("model-router-x"), None);
    let mut call = args(Some(&view));
    call.billing.model = Some("model-router-x");
    call.billing.custom_llm_provider = Some("azure_ai");
    let charge = rig(&[
        ("model-router-x", GPT),
        ("azure_ai/model-router-x", GPT),
        ("model_router", ROUTED),
    ])
    .charge(&call, None::<&mut Stored>)
    .await
    .unwrap();
    assert_eq!(charge, dec("0.02"));
}

#[tokio::test]
async fn a_call_without_a_response_records_no_token_type_costs() {
    let mut stored = Stored::default();
    let charge = rig(&[("gpt-4o", GPT)])
        .charge(&args(None), Some(&mut stored))
        .await
        .unwrap();
    assert_eq!(charge, dec("0"));
    assert_eq!(stored.0.unwrap().token_costs, [None, None, None]);
}

#[tokio::test]
async fn an_ocr_page_is_priced_at_the_deployments_own_rate_when_one_is_given() {
    let view = ResponseView::Other;
    let billing = OcrBilling {
        kind: OcrResponseKind::Ocr,
        usage: Some(OcrUsage {
            credits: None,
            pages_processed: Some(10),
            pages_processed_annotation: 0,
        }),
    };
    let catalog: &[(&str, &str)] = &[
        ("input_cost_per_token", "0"),
        ("output_cost_per_token", "0"),
        ("ocr_cost_per_page", "4"),
    ];
    let own = map(r#"{"ocr_cost_per_page":2}"#);
    let priced = rig(&[("oracle-ocr", catalog)]);
    let mut call = args(Some(&view));
    call.billing.model = Some("oracle-ocr");
    call.billing.call_type = Some(CallType::Ocr);
    call.charge.ocr_billing = Some(&billing);
    let plain = priced.charge(&call, None::<&mut Stored>).await.unwrap();
    assert_eq!(plain, dec("40"));
    call.billing.custom_pricing = Some(true);
    call.charge.logging.params = Some(&own);
    let overridden = priced.charge(&call, None::<&mut Stored>).await.unwrap();
    assert_eq!(overridden, dec("20"));
}

#[tokio::test]
async fn a_setter_failure_never_replaces_the_charge() {
    let usage = chat_usage(10, 5);
    let view = chat_view(&usage, Some("gpt-4o"), None);
    let charge = rig(&[("gpt-4o", GPT)])
        .charge(&args(Some(&view)), Some(&mut Refusing))
        .await;
    assert_eq!(charge.unwrap(), dec("0.02"));
}

#[tokio::test]
async fn an_explicit_price_suppresses_the_regional_entry() {
    let usage = chat_usage(10, 5);
    let view = chat_view(&usage, Some("gpt-4o"), None);
    let regional: &[(&str, &str)] = &[
        ("input_cost_per_token", "0.01"),
        ("output_cost_per_token", "0.02"),
    ];
    let priced = rig(&[("gpt-4o", GPT), ("openai/eu/gpt-4o", regional)]);
    let mut call = args(Some(&view));
    call.billing.region_name = Some("eu");
    let regional_charge = priced.charge(&call, None::<&mut Stored>).await.unwrap();
    assert_eq!(regional_charge, dec("0.2"));
    call.billing.custom_pricing = Some(true);
    let explicit_charge = priced.charge(&call, None::<&mut Stored>).await.unwrap();
    assert_eq!(explicit_charge, dec("0.02"));
}

fn billable_event(service_tier: &str, prompt: &str, completion: &str) -> IndexMap<String, Json> {
    map(&format!(
        r#"{{"type":"response.completed","response":{{"usage":{{"prompt_tokens":{prompt},"completion_tokens":{completion}}},"service_tier":"{service_tier}"}}}}"#
    ))
}

#[tokio::test]
async fn websocket_partitions_are_priced_apart_and_summed() {
    let record = RealtimeBilling {
        results: vec![
            billable_event("priority", "10", "2"),
            billable_event("flex", "4", "1"),
        ],
        usage: SumUsage::default(),
        service_tier: None,
    };
    let usage = chat_usage(14, 3);
    let view = ResponseView::Realtime {
        record: &record,
        usage: &usage,
    };
    let mut call = args(Some(&view));
    call.billing.call_type = Some(CallType::PrivateAresponsesWebsocket);
    let mut stored = Stored::default();
    let charge = rig(&[("gpt-4o", GPT)])
        .charge(&call, Some(&mut stored))
        .await
        .unwrap();
    assert_eq!(charge, dec("0.02"));
    // The last partition's breakdown replaces the earlier one.
    let recorded = stored.0.unwrap();
    assert_eq!(recorded.input_cost, dec("0.004"));
    assert_eq!(recorded.output_cost, dec("0.002"));
}

#[tokio::test]
async fn a_realtime_session_is_priced_from_its_own_stream_not_the_token_stage() {
    let record = RealtimeBilling {
        results: vec![map(
            r#"{"type":"session.created","session":{"model":"gpt-4o"}}"#,
        )],
        usage: SumUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
            ..SumUsage::default()
        },
        service_tier: None,
    };
    let usage = chat_usage(10, 5);
    let view = ResponseView::Realtime {
        record: &record,
        usage: &usage,
    };
    let mut call = args(Some(&view));
    call.billing.call_type = Some(CallType::PrivateArealtime);
    let mut stored = Stored::default();
    let charge = rig(&[("gpt-4o", GPT)])
        .charge(&call, Some(&mut stored))
        .await
        .unwrap();
    assert_eq!(charge, dec("0.02"));
    // The token stage records the pre-adjustment cost, the realtime stream does not.
    assert!(stored.0.unwrap().original_cost.is_none());
}

#[test]
fn the_clock_reading_is_a_utc_moment_and_never_before_the_epoch() {
    let mut later = || UNIX_EPOCH + Duration::new(1_700_000_000, 5);
    let moment = reading(&mut later).unwrap();
    assert_eq!(moment.timestamp(), 1_700_000_000);
    assert_eq!(moment.timestamp_subsec_nanos(), 5);
    let mut earlier = || UNIX_EPOCH - Duration::from_secs(1);
    assert!(matches!(
        reading(&mut earlier),
        Err(ChargeInputError::Clock)
    ));
}
