use std::collections::BTreeMap;

use betterlite_config::cost_adjustments::{AdjustmentEntry, MarginFieldEntry};
use betterlite_logging::fields::usage::Usage;
use betterlite_models::cost_usage::transcription::{
    TranscriptionTokenDetails, TranscriptionTokens, TranscriptionUsage,
};
use betterlite_models::decisions::Json;
use betterlite_models::protocol_cost::{A2aCall, A2aUsage, ProtocolCall};
use betterlite_models::tool_fees::billing::BuiltInToolParams;
use betterlite_models::tool_signals::kwargs::WebSearchOptions;
use betterlite_models::tool_signals::response::{
    Annotation, ChatChoice, ChatMessage, ChatView, OtherView, ToolBillingResponse,
};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

use super::{Entry, Input, Kind, Margin, UsageInput};

pub(crate) fn usage_block(fields: &UsageInput) -> Usage {
    Usage {
        prompt_tokens: fields.prompt,
        completion_tokens: fields.completion,
        total_tokens: fields.prompt.saturating_add(fields.completion),
        prompt_tokens_details: fields.cached.map(|count| {
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

pub(crate) fn adjustments(
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

pub(crate) fn hidden_params(header: &Number) -> IndexMap<String, Json> {
    let headers = IndexMap::from([(
        "llm_provider-x-litellm-response-cost".to_owned(),
        Json::Number(header.clone()),
    )]);
    IndexMap::from([("additional_headers".to_owned(), Json::Object(headers))])
}

pub(crate) fn search_rates(catalog: &BTreeMap<String, Entry>) -> BTreeMap<String, Decimal> {
    let mut rates = BTreeMap::new();
    for (model, entry) in catalog {
        if let Some(rate) = entry
            .input_cost_per_query
            .as_ref()
            .and_then(|number| number.to_string().parse().ok())
        {
            rates.insert(model.clone(), rate);
        }
    }
    rates
}

pub(crate) fn transcription_usage(input: &Input) -> TranscriptionUsage {
    let prompt = input.usage.prompt;
    let completion = input.usage.completion;
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

pub(crate) fn transcription_duration(input: &Input) -> Option<Decimal> {
    let number = input.transcription_seconds.as_ref()?;
    number.to_string().parse().ok()
}

pub(crate) fn protocol(input: &Input) -> Option<ProtocolCall> {
    let agent = input.agent.as_ref()?;
    Some(ProtocolCall::A2a(A2aCall {
        response_cost: agent
            .response_cost
            .as_ref()
            .map_or(Json::Null, |number| Json::Number(number.clone())),
        cost_per_query: agent
            .cost_per_query
            .as_ref()
            .map_or(Json::Null, |number| Json::Number(number.clone())),
        input_cost_per_token: Json::Null,
        output_cost_per_token: Json::Null,
        usage: A2aUsage::Absent,
    }))
}

pub(crate) fn tool_params(input: &Input) -> BuiltInToolParams {
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
        file_search: None,
        vector_store_usage: None,
        computer_use_usage: None,
        code_interpreter_sessions: None,
    }
}

pub(crate) fn tool_response(input: &Input) -> ToolBillingResponse {
    if input.response_kind.unwrap_or(Kind::Chat) != Kind::Chat {
        return ToolBillingResponse::Other(OtherView { usage: None });
    }
    let annotations = (input.annotations == Some(true)).then(|| {
        vec![Annotation {
            annotation_type: Some(Json::Text("url_citation".to_owned())),
        }]
    });
    ToolBillingResponse::Chat(ChatView {
        choices: vec![ChatChoice {
            message: Some(ChatMessage { annotations }),
        }],
        usage: None,
    })
}
