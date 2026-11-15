use betterlite_core::cost::call_price::CustomCostPerToken;
use betterlite_core::cost::completion::args::{ChargeInputs, CompletionArgs, ToolInputs};
use betterlite_core::cost::prepare::{CompletionBillingInput, ResponseView};
use betterlite_core::cost::realtime::RealtimeBilling;
use betterlite_core::cost::special::request::{HiddenDuration, LoggingView, TranscriptionDuration};
use betterlite_logging::fields::usage::Usage;
use betterlite_models::cost_usage::transcription::TranscriptionUsage;
use betterlite_models::decisions::Json;
use betterlite_models::image_cost::ImageBilling;
use betterlite_models::ocr_pricing::OcrBilling;
use betterlite_models::protocol_cost::ProtocolCall;
use indexmap::IndexMap;

use super::input::Input;
use super::input::{ocr, realtime, values};
use super::views::response_parts;

pub(super) struct Parts {
    pub(super) usage: Usage,
    pub(super) hidden: Option<IndexMap<String, Json>>,
    pub(super) transcription_usage: TranscriptionUsage,
    pub(super) transcription: TranscriptionDuration,
    pub(super) image_billing: ImageBilling,
    ocr_billing: Option<OcrBilling>,
    pub(super) protocol: Option<ProtocolCall>,
    pub(super) realtime: RealtimeBilling,
}

pub(super) fn build_parts(input: &Input) -> Parts {
    Parts {
        usage: values::usage_block(&input.usage),
        hidden: input.header_cost.as_ref().map(values::hidden_params),
        transcription_usage: values::transcription_usage(input),
        transcription: TranscriptionDuration {
            hidden: HiddenDuration::Missing,
            response_duration: input
                .transcription_seconds
                .as_ref()
                .and_then(|number| number.to_string().parse().ok()),
        },
        image_billing: ImageBilling {
            image_count: 1,
            size: None,
            quality: None,
            usage: None,
        },
        ocr_billing: ocr::billing(input),
        protocol: values::protocol(input),
        realtime: RealtimeBilling {
            results: input.events.clone().unwrap_or_default(),
            usage: realtime::usage(&input.usage),
            service_tier: None,
        },
    }
}

fn custom_cost_per_token(input: &Input) -> Option<CustomCostPerToken> {
    let rates = input.custom_price.as_ref()?;
    Some(CustomCostPerToken {
        input_cost_per_token: rates.input_cost_per_token,
        output_cost_per_token: rates.output_cost_per_token,
        cache_read_input_token_cost: None,
        cache_creation_cost: None,
    })
}

fn charge_inputs<'input>(input: &'input Input, parts: &'input Parts) -> ChargeInputs<'input> {
    ChargeInputs {
        passthrough_image: false,
        total_time: None,
        data_residency: None,
        vertex_location: None,
        custom_cost_per_token: custom_cost_per_token(input),
        custom_cost_per_second: None,
        logging: LoggingView {
            model: input.logging_model.as_deref(),
            params: None,
            timing: None,
        },
        response: response_parts(input, parts),
        ocr_billing: parts.ocr_billing.as_ref(),
        vector_response: None,
        search_hidden_params: None,
        server_tool_projection: None,
        tools: ToolInputs {
            response: values::tool_response(input),
            params: values::tool_params(input),
            google_maps_requests: None,
        },
    }
}

pub(super) fn completion_args<'input>(
    input: &'input Input,
    parts: &'input Parts,
    view: &'input ResponseView<'input>,
) -> CompletionArgs<'input> {
    CompletionArgs {
        billing: CompletionBillingInput {
            model: Some(input.model.as_str()),
            prompt: "",
            messages: &[],
            completion: "",
            call_type: input.call_type,
            custom_llm_provider: input.custom_llm_provider.as_deref(),
            region_name: input.region_name.as_deref(),
            router_model_id: None,
            custom_pricing: input.custom_pricing,
            base_model: None,
            optional_params: input.optional_params.as_ref(),
            service_tier: input.service_tier.as_deref(),
            response: Some(view),
        },
        charge: charge_inputs(input, parts),
    }
}
