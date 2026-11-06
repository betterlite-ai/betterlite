use crate::presidio_actions::request_data::RequestData;
use crate::presidio_response::anthropic::process_anthropic_response;
use crate::presidio_response::error::PresidioResponseError;
use crate::presidio_response::model::process_response_for_pii;
use crate::presidio_response::wire::ResponseBody;
use crate::presidio_response::{HookContext, Mode};

pub async fn async_post_call_success_hook(
    context: HookContext<'_>,
    request_data: &mut RequestData,
    response: &mut ResponseBody,
) -> Result<(), PresidioResponseError> {
    if context.analyzer.settings.apply_to_output {
        if let ResponseBody::Anthropic(message) = response {
            return process_anthropic_response(context, message, request_data, Mode::Mask).await;
        }
        return mask_output_response(context, response, request_data).await;
    }
    if !context.analyzer.settings.output_parse_pii && !context.global_output_parse_pii {
        return Ok(());
    }
    match response {
        ResponseBody::Chat(chat) => {
            process_response_for_pii(context, chat, request_data, Mode::Unmask).await
        }
        ResponseBody::Anthropic(message) => {
            process_anthropic_response(context, message, request_data, Mode::Unmask).await
        }
        ResponseBody::Stream(_) | ResponseBody::Other(_) => Ok(()),
    }
}

pub async fn mask_output_response(
    context: HookContext<'_>,
    response: &mut ResponseBody,
    request_data: &mut RequestData,
) -> Result<(), PresidioResponseError> {
    match response {
        ResponseBody::Chat(chat) => {
            process_response_for_pii(context, chat, request_data, Mode::Mask).await
        }
        ResponseBody::Stream(_) | ResponseBody::Anthropic(_) | ResponseBody::Other(_) => Ok(()),
    }
}
