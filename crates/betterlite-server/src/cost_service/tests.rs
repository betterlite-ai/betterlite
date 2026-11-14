#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::str::FromStr as _;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_config::cost_adjustments::{AdjustmentEntry, Settings as AdjustmentSettings};
use betterlite_core::cost::completion::args::{
    ChargeInputs, CompletionArgs, ResponseParts, ToolInputs,
};
use betterlite_core::cost::completion::lookup::LookupSettings;
use betterlite_core::cost::image_dispatch::policy::ImageResponseView;
use betterlite_core::cost::prepare::{
    BillingTokenCounter, CompletionBillingInput, ResponseView, TypedResponse,
};
use betterlite_core::cost::special::replicate::ReplicateStamps;
use betterlite_core::cost::special::request::{ImageParts, LoggingView, TranscriptionDuration};
use betterlite_core::cost::usage::{BillingUsage, ResponseKind};
use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use betterlite_logging::fields::usage::Usage;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::tool_fees::billing::BuiltInToolParams;
use betterlite_models::tool_signals::response::{OtherView, ToolBillingResponse};
use betterlite_provider::pricing::grounding::ProviderFormulas;
use betterlite_tokenizer::count::error::CountError;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::{Call, CostService, Policies, name_normalizers};

const CATALOG: &str = r#"{
    "gpt-4o": {
        "litellm_provider": "openai",
        "mode": "chat",
        "input_cost_per_token": 0.001,
        "output_cost_per_token": 0.002
    }
}"#;

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("the fixture is a decimal")
}

fn clock() -> SystemTime {
    UNIX_EPOCH
}

fn service(adjustments: &AdjustmentSettings) -> CostService<ProviderFormulas> {
    let settings = LookupSettings {
        known_model_names: BTreeSet::new(),
        custom_providers: BTreeSet::new(),
        environment: BTreeMap::new(),
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
        normalizers: name_normalizers(),
    };
    CostService::new(
        CATALOG.as_bytes(),
        settings,
        adjustments,
        Policies::native(),
    )
    .expect("the fixture catalog parses")
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

#[derive(Default)]
struct Stored(Option<Decimal>);

impl CostBreakdownTarget for Stored {
    type Failure = Infallible;

    fn set_cost_breakdown(&mut self, input: &CostBreakdownInput) -> Result<(), Infallible> {
        self.0 = Some(input.total_cost);
        Ok(())
    }
}

fn usage(prompt: i64, completion: i64) -> Usage {
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: prompt + completion,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        server_tool_use: None,
        cost: None,
        extras: IndexMap::new(),
    }
}

fn view<'view>(
    usage: &'view Usage,
    hidden_params: Option<&'view IndexMap<String, Json>>,
) -> ResponseView<'view> {
    ResponseView::Typed(TypedResponse {
        kind: ResponseKind::Chat,
        model: Some("gpt-4o"),
        usage: BillingUsage::Chat(usage),
        service_tier: None,
        hidden_params,
    })
}

fn args<'args>(response: &'args ResponseView<'args>) -> CompletionArgs<'args> {
    CompletionArgs {
        billing: CompletionBillingInput {
            model: Some("gpt-4o"),
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
            response: Some(response),
        },
        charge: ChargeInputs {
            passthrough_image: false,
            total_time: None,
            data_residency: None,
            vertex_location: None,
            custom_cost_per_token: None,
            custom_cost_per_second: None,
            logging: LoggingView {
                model: None,
                params: None,
                timing: None,
            },
            response: ResponseParts {
                cost_response: None,
                image: ImageParts {
                    response: ImageResponseView::Other,
                    quality: None,
                    image_count: None,
                    size: None,
                },
                video_usage: None,
                transcription: TranscriptionDuration::default(),
                rerank: None,
                protocol: None,
                replicate: ReplicateStamps::default(),
                model_response: None,
            },
            ocr_billing: None,
            vector_response: None,
            search_hidden_params: None,
            server_tool_projection: None,
            tools: ToolInputs {
                response: ToolBillingResponse::Other(OtherView { usage: None }),
                params: BuiltInToolParams::default(),
                google_maps_requests: None,
            },
        },
    }
}

fn header() -> IndexMap<String, Json> {
    let found: Json = serde_json::from_str(
        r#"{"additional_headers": {"llm_provider-x-litellm-response-cost": 0.5}}"#,
    )
    .expect("the fixture is JSON");
    let Json::Object(fields) = found else {
        panic!("the fixture is an object");
    };
    fields
}

#[tokio::test]
async fn a_chat_call_is_priced_through_the_effective_lookup() {
    let service = service(&AdjustmentSettings::default());
    let usage = usage(10, 5);
    let response = view(&usage, None);
    let args = args(&response);
    let call = Call {
        args: &args,
        counter: &Counter,
    };
    let mut stored = Stored::default();
    let charge = service
        .completion_cost(&call, &mut clock, Some(&mut stored))
        .await
        .unwrap();
    assert_eq!(charge, dec("0.02"));
    assert_eq!(stored.0, Some(dec("0.02")));
}

#[tokio::test]
async fn the_configured_discount_reaches_the_charge() {
    let mut adjustments = AdjustmentSettings::default();
    adjustments
        .cost_discount_config
        .insert("openai".to_owned(), AdjustmentEntry::Percent(dec("0.5")));
    let service = service(&adjustments);
    let usage = usage(10, 5);
    let response = view(&usage, None);
    let args = args(&response);
    let call = Call {
        args: &args,
        counter: &Counter,
    };
    let charge = service
        .completion_cost::<Stored, _>(&call, &mut clock, None)
        .await
        .unwrap();
    assert_eq!(charge, dec("0.01"));
}

#[tokio::test]
async fn a_cache_hit_costs_nothing() {
    let service = service(&AdjustmentSettings::default());
    let usage = usage(10, 5);
    let response = view(&usage, None);
    let args = args(&response);
    let call = Call {
        args: &args,
        counter: &Counter,
    };
    let charge = service
        .response_cost::<Stored, _>(&call, Some(true), &mut clock, None)
        .await
        .unwrap();
    assert_eq!(charge, dec("0"));
}

#[tokio::test]
async fn the_provider_header_wins_over_the_token_prices() {
    let service = service(&AdjustmentSettings::default());
    let usage = usage(10, 5);
    let hidden = header();
    let response = view(&usage, Some(&hidden));
    let args = args(&response);
    let call = Call {
        args: &args,
        counter: &Counter,
    };
    let charge = service
        .response_cost::<Stored, _>(&call, Some(false), &mut clock, None)
        .await
        .unwrap();
    assert_eq!(charge, dec("0.5"));
}
