use serde_json::value::RawValue;

use crate::presidio_actions::check::CheckPiiCall;
use crate::presidio_actions::request_data::{
    RequestData, pii_tokens, presidio_settings_from_request,
};
use crate::presidio_actions::tokens::unmask_pii_text;
use crate::presidio_response::error::PresidioResponseError;
use crate::presidio_response::wire::{AnthropicMessage, ContentItem, Extensions};
use crate::presidio_response::{HookContext, Mode};

#[must_use]
pub fn is_anthropic_message_response(raw: &RawValue) -> bool {
    let Ok(fields) = serde_json::from_str::<Extensions>(raw.get()) else {
        return false;
    };
    let is_message = fields
        .get("type")
        .and_then(|value| serde_json::from_str::<String>(value.get()).ok())
        .is_some_and(|kind| kind == "message");
    is_message
        && fields
            .get("content")
            .is_some_and(|value| value.get().trim_start().starts_with('['))
}

pub async fn process_anthropic_response(
    context: HookContext<'_>,
    response: &mut AnthropicMessage,
    request_data: &mut RequestData,
    mode: Mode,
) -> Result<(), PresidioResponseError> {
    let tokens = pii_tokens(request_data)?;
    if tokens.is_empty() && mode == Mode::Unmask {
        tracing::debug!("Presidio Anthropic response unmask found no pii_tokens in metadata");
    }
    let presidio_config = presidio_settings_from_request(request_data)?;
    let Some(blocks) = response.content.as_mut() else {
        return Ok(());
    };
    for item in blocks {
        let ContentItem::Object(block) = item else {
            continue;
        };
        if block.kind.as_deref() == Some("text")
            && let Some(text) = block.text.clone()
        {
            let masked = match mode {
                Mode::Unmask => unmask_pii_text(&text, &tokens),
                Mode::Mask => {
                    context
                        .anonymizer
                        .check_pii(
                            context.analyzer,
                            CheckPiiCall {
                                text: &text,
                                output_parse_pii: false,
                                presidio_config: presidio_config.as_ref(),
                                dynamic_params: context.dynamic_params,
                                pii_entities_config: context.pii_entities_config,
                                request_data: &mut *request_data,
                            },
                        )
                        .await?
                }
            };
            block.set_text(masked);
        }
    }
    Ok(())
}
