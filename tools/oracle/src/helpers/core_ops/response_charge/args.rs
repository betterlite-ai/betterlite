use betterlite_core::cost::call_price::CustomCostPerToken;
use betterlite_core::cost::completion::args::{
    ChargeInputs, CompletionArgs, ResponseParts, ToolInputs,
};
use betterlite_core::cost::image_dispatch::policy::ImageResponseView;
use betterlite_core::cost::prepare::{CompletionBillingInput, ResponseView, TypedResponse};
use betterlite_core::cost::realtime::RealtimeBilling;
use betterlite_core::cost::special::replicate::ReplicateStamps;
use betterlite_core::cost::special::request::{
    HiddenDuration, ImageParts, LoggingView, TranscriptionDuration,
};
use betterlite_core::cost::usage::{BillingUsage, ResponseKind};
use betterlite_logging::fields::usage::Usage;
use betterlite_models::cost_usage::transcription::TranscriptionUsage;
use betterlite_models::decisions::Json;
use betterlite_models::image_cost::ImageBilling;
use betterlite_models::ocr_pricing::OcrBilling;
use betterlite_models::protocol_cost::ProtocolCall;
use indexmap::IndexMap;

use super::input::values;
use super::input::{Input, Kind};

pub(super) struct Parts {
    usage: Usage,
    hidden: Option<IndexMap<String, Json>>,
    transcription_usage: TranscriptionUsage,
    transcription: TranscriptionDuration,
    image_billing: ImageBilling,
    ocr_billing: Option<OcrBilling>,
    protocol: Option<ProtocolCall>,
    realtime: RealtimeBilling,
}

pub(super) fn build_parts(input: &Input) -> Parts {
    Parts {
        usage: values::usage_block(&input.usage),
        hidden: input.header_cost.as_ref().map(values::hidden_params),
        transcription_usage: values::transcription_usage(input),
        transcription: TranscriptionDuration {
            hidden: HiddenDuration::Missing,
            response_duration: values::transcription_duration(input),
        },
        image_billing: ImageBilling {
            image_count: 1,
            size: None,
            quality: None,
            usage: None,
        },
        ocr_billing: values::ocr_billing(input),
        protocol: values::protocol(input),
        realtime: RealtimeBilling {
            results: input.events.clone().unwrap_or_default(),
            usage: values::pricing_usage(&input.usage),
            service_tier: None,
        },
    }
}

pub(super) fn response_view<'view>(
    input: &'view Input,
    parts: &'view Parts,
) -> ResponseView<'view> {
    match input.response_kind.unwrap_or(Kind::Chat) {
        Kind::Chat => ResponseView::Typed(TypedResponse {
            kind: ResponseKind::Chat,
            model: input.response_model.as_deref(),
            usage: BillingUsage::Chat(&parts.usage),
            service_tier: None,
            hidden_params: parts.hidden.as_ref(),
        }),
        Kind::Image => ResponseView::Typed(TypedResponse {
            kind: ResponseKind::Image,
            model: input.response_model.as_deref(),
            usage: BillingUsage::Absent,
            service_tier: None,
            hidden_params: None,
        }),
        Kind::Transcription => ResponseView::Typed(TypedResponse {
            kind: ResponseKind::Transcription,
            model: input.response_model.as_deref(),
            usage: BillingUsage::Transcription(&parts.transcription_usage),
            service_tier: None,
            hidden_params: None,
        }),
        Kind::RealtimeStream => ResponseView::Realtime {
            record: &parts.realtime,
            usage: &parts.usage,
        },
        Kind::Ocr | Kind::None => ResponseView::Other,
    }
}

fn response_parts<'input>(input: &'input Input, parts: &'input Parts) -> ResponseParts<'input> {
    let image_response = if input.response_kind.unwrap_or(Kind::Chat) == Kind::Image {
        ImageResponseView::Image(&parts.image_billing)
    } else {
        ImageResponseView::Other
    };
    ResponseParts {
        cost_response: None,
        image: ImageParts {
            response: image_response,
            quality: None,
            image_count: None,
            size: None,
        },
        video_usage: None,
        transcription: parts.transcription,
        rerank: None,
        protocol: parts.protocol.as_ref(),
        replicate: ReplicateStamps::default(),
        model_response: None,
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
