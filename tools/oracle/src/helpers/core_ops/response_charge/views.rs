use betterlite_core::cost::completion::args::ResponseParts;
use betterlite_core::cost::image_dispatch::policy::ImageResponseView;
use betterlite_core::cost::prepare::{ResponseView, TypedResponse};
use betterlite_core::cost::special::replicate::ReplicateStamps;
use betterlite_core::cost::special::request::ImageParts;
use betterlite_core::cost::usage::{BillingUsage, ResponseKind};

use super::args::Parts;
use super::input::{Input, Kind};

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

pub(super) fn response_parts<'input>(
    input: &'input Input,
    parts: &'input Parts,
) -> ResponseParts<'input> {
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
