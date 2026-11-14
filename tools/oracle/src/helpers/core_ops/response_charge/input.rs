use std::collections::BTreeMap;

use betterlite_config::cost_adjustments::{AdjustmentEntry, MarginFieldEntry};
use betterlite_core::cost::realtime::Event;
use betterlite_core::routes::CallType;
use betterlite_logging::fields::usage::Usage;
use betterlite_models::cost_usage::transcription::{
    TranscriptionTokenDetails, TranscriptionTokens, TranscriptionUsage,
};
use betterlite_models::decisions::Json;
use betterlite_models::ocr_pricing::{OcrBilling, OcrResponseKind, OcrUsage};
use betterlite_models::protocol_cost::{A2aCall, A2aUsage, ProtocolCall};
use betterlite_models::tool_fees::billing::BuiltInToolParams;
use betterlite_models::tool_signals::kwargs::WebSearchOptions;
use betterlite_models::tool_signals::response::{
    Annotation, ChatChoice, ChatMessage, ChatView, OtherView, ToolBillingResponse,
};
use betterlite_models::usage_sum::Usage as PricingUsage;
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

#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind {
    Chat,
    Image,
    Transcription,
    Ocr,
    RealtimeStream,
    None,
}

impl Kind {
    pub(super) fn of(input: &Input) -> Self {
        input.response_kind.unwrap_or(Self::Chat)
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) input_cost_per_image: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) input_cost_per_query: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) ocr_cost_per_credit: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) ocr_cost_per_page: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) output_cost_per_second: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) input_cost_per_token_priority: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) output_cost_per_token_priority: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) input_cost_per_token_flex: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) output_cost_per_token_flex: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) search_context_cost_per_query: Option<BTreeMap<String, Number>>,
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
pub(super) struct OcrInput {
    pub(super) credits: Option<Number>,
    pub(super) pages_processed: Option<i64>,
    pub(super) pages_processed_annotation: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AgentInput {
    pub(super) response_cost: Option<Number>,
    pub(super) cost_per_query: Option<Number>,
}

// Only the keys the fixtures send; an unknown built-in tool param is a case-authoring error.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StandardTools {
    pub(super) web_search_options: Option<BTreeMap<String, Json>>,
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
    pub(super) call_type: Option<CallType>,
    pub(super) response_kind: Option<Kind>,
    pub(super) response_model: Option<String>,
    pub(super) optional_params: Option<IndexMap<String, Json>>,
    pub(super) standard_tools: Option<StandardTools>,
    pub(super) annotations: Option<bool>,
    pub(super) logging_model: Option<String>,
    pub(super) agent: Option<AgentInput>,
    pub(super) refuse_breakdown: Option<bool>,
    pub(super) custom_pricing: Option<bool>,
    pub(super) service_tier: Option<String>,
    pub(super) transcription_seconds: Option<Number>,
    pub(super) ocr: Option<OcrInput>,
    pub(super) events: Option<Vec<IndexMap<String, Json>>>,
}

pub(super) fn number_decimal(number: &Number) -> Option<Decimal> {
    number.to_string().parse().ok()
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

pub(super) fn pricing_usage(fields: &UsageInput) -> PricingUsage {
    PricingUsage {
        prompt_tokens: fields.prompt_tokens,
        completion_tokens: fields.completion_tokens,
        total_tokens: fields
            .prompt_tokens
            .saturating_add(fields.completion_tokens),
        ..PricingUsage::default()
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

pub(super) fn search_rates(catalog: &BTreeMap<String, Entry>) -> BTreeMap<String, Decimal> {
    let mut rates = BTreeMap::new();
    for (model, entry) in catalog {
        if let Some(rate) = entry.input_cost_per_query.as_ref().and_then(number_decimal) {
            rates.insert(model.clone(), rate);
        }
    }
    rates
}

pub(super) fn transcription_usage(input: &Input) -> TranscriptionUsage {
    let prompt = input.usage.prompt_tokens;
    let completion = input.usage.completion_tokens;
    TranscriptionUsage::Tokens(Box::new(TranscriptionTokens {
        input_tokens: prompt,
        output_tokens: completion,
        total_tokens: prompt.saturating_add(completion),
        input_token_details: TranscriptionTokenDetails {
            audio_tokens: 0,
            text_tokens: prompt,
        },
    }))
}

pub(super) fn transcription_duration(input: &Input) -> Option<Decimal> {
    input
        .transcription_seconds
        .as_ref()
        .and_then(number_decimal)
}

pub(super) fn ocr_billing(input: &Input) -> Option<OcrBilling> {
    let usage = input.ocr.as_ref()?;
    Some(OcrBilling {
        kind: OcrResponseKind::Ocr,
        usage: Some(OcrUsage {
            credits: usage.credits.as_ref().and_then(number_decimal),
            pages_processed: usage.pages_processed,
            pages_processed_annotation: usage.pages_processed_annotation.unwrap_or(0),
        }),
    })
}

fn money(value: Option<&Number>) -> Json {
    value.map_or(Json::Null, |number| Json::Number(number.clone()))
}

pub(super) fn protocol(input: &Input) -> Option<ProtocolCall> {
    let agent = input.agent.as_ref()?;
    Some(ProtocolCall::A2a(A2aCall {
        response_cost: money(agent.response_cost.as_ref()),
        cost_per_query: money(agent.cost_per_query.as_ref()),
        input_cost_per_token: Json::Null,
        output_cost_per_token: Json::Null,
        usage: A2aUsage::Absent,
    }))
}

pub(super) fn tool_params(input: &Input) -> BuiltInToolParams {
    let Some(tools) = input.standard_tools.as_ref() else {
        return BuiltInToolParams::default();
    };
    let web_search_options = tools
        .web_search_options
        .as_ref()
        .map(|fields| WebSearchOptions {
            fields: fields
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        });
    BuiltInToolParams {
        web_search_options,
        ..BuiltInToolParams::default()
    }
}

pub(super) fn tool_response(input: &Input) -> ToolBillingResponse {
    if Kind::of(input) != Kind::Chat {
        return ToolBillingResponse::Other(OtherView { usage: None });
    }
    let annotations = if input.annotations == Some(true) {
        Some(vec![Annotation {
            annotation_type: Some(Json::Text("url_citation".to_owned())),
        }])
    } else {
        None
    };
    ToolBillingResponse::Chat(ChatView {
        choices: vec![ChatChoice {
            message: Some(ChatMessage { annotations }),
        }],
        usage: None,
    })
}

pub(super) fn events(input: &Input) -> Vec<Event> {
    input.events.clone().unwrap_or_default()
}
