use std::collections::BTreeMap;

use betterlite_config::cost_adjustments::{AdjustmentEntry, MarginFieldEntry};
use betterlite_core::cost::call_price::CustomCostPerToken;
use betterlite_core::cost::completion::args::{ChargeInputs, ResponseParts, ToolInputs};
use betterlite_core::cost::image_dispatch::policy::ImageResponseView;
use betterlite_core::cost::special::replicate::ReplicateStamps;
use betterlite_core::cost::special::request::{ImageParts, LoggingView, TranscriptionDuration};
use betterlite_logging::fields::usage::Usage;
use betterlite_models::decisions::Json;
use betterlite_models::tool_fees::billing::BuiltInToolParams;
use betterlite_models::tool_signals::response::{OtherView, ToolBillingResponse};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Number;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Function {
    CompletionCost,
    ResponseCostCalculator,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub(super) litellm_provider: String,
    pub(super) mode: String,
    pub(super) input_cost_per_token: Number,
    pub(super) output_cost_per_token: Number,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cache_read_input_token_cost: Option<Number>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UsageInput {
    pub(super) prompt_tokens: i64,
    pub(super) completion_tokens: i64,
    pub(super) cached_tokens: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rates {
    pub(super) input_cost_per_token: Decimal,
    pub(super) output_cost_per_token: Decimal,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Margin {
    Percent(Decimal),
    Split {
        percentage: Option<Decimal>,
        fixed_amount: Option<Decimal>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) function: Function,
    pub(super) model: String,
    pub(super) custom_llm_provider: Option<String>,
    pub(super) region_name: Option<String>,
    pub(super) catalog: BTreeMap<String, Entry>,
    pub(super) usage: UsageInput,
    pub(super) custom_price: Option<Rates>,
    pub(super) cache_hit: Option<bool>,
    pub(super) header_cost: Option<Number>,
    pub(super) discount: BTreeMap<String, Decimal>,
    pub(super) margin: BTreeMap<String, Margin>,
}

pub(super) fn usage_block(fields: &UsageInput) -> Usage {
    Usage {
        prompt_tokens: fields.prompt_tokens,
        completion_tokens: fields.completion_tokens,
        total_tokens: fields
            .prompt_tokens
            .saturating_add(fields.completion_tokens),
        prompt_tokens_details: fields.cached_tokens.map(|count| {
            IndexMap::from([(
                "cached_tokens".to_owned(),
                Json::Number(Number::from(count)),
            )])
        }),
        completion_tokens_details: None,
        server_tool_use: None,
        cost: None,
        extras: IndexMap::new(),
    }
}

pub(super) fn adjustments(
    input: &Input,
) -> (
    BTreeMap<String, AdjustmentEntry>,
    BTreeMap<String, AdjustmentEntry>,
) {
    let discount = input
        .discount
        .iter()
        .map(|(provider, percent)| (provider.clone(), AdjustmentEntry::Percent(*percent)))
        .collect();
    let field =
        |value: Option<Decimal>| value.map_or(MarginFieldEntry::Absent, MarginFieldEntry::Number);
    let margin = input
        .margin
        .iter()
        .map(|(provider, entry)| {
            let found = match entry {
                Margin::Percent(percent) => AdjustmentEntry::Percent(*percent),
                Margin::Split {
                    percentage,
                    fixed_amount,
                } => AdjustmentEntry::Split {
                    percentage: field(*percentage),
                    fixed_amount: field(*fixed_amount),
                },
            };
            (provider.clone(), found)
        })
        .collect();
    (discount, margin)
}

pub(super) fn hidden_params(header: &Number) -> IndexMap<String, Json> {
    let headers = IndexMap::from([(
        "llm_provider-x-litellm-response-cost".to_owned(),
        Json::Number(header.clone()),
    )]);
    IndexMap::from([("additional_headers".to_owned(), Json::Object(headers))])
}

pub(super) fn charge_inputs(input: &Input) -> ChargeInputs<'_> {
    ChargeInputs {
        passthrough_image: false,
        total_time: None,
        data_residency: None,
        vertex_location: None,
        custom_cost_per_token: input.custom_price.as_ref().map(|rates| CustomCostPerToken {
            input_cost_per_token: rates.input_cost_per_token,
            output_cost_per_token: rates.output_cost_per_token,
            cache_read_input_token_cost: None,
            cache_creation_cost: None,
        }),
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
    }
}
