use indexmap::IndexMap;

use crate::presidio_actions::check::CheckPiiCall;
use crate::presidio_actions::request_data::{
    PresidioPerRequestConfig, RequestData, pii_tokens, presidio_settings_from_request,
};
use crate::presidio_actions::tokens::unmask_pii_text;
use crate::presidio_response::error::PresidioResponseError;
use crate::presidio_response::wire::{
    ChatModelResponse, ChatResponseMessage, Content, ContentItem, Extensions, ToolCallItem,
};
use crate::presidio_response::{HookContext, Mode};

pub async fn process_response_for_pii(
    context: HookContext<'_>,
    response: &mut ChatModelResponse,
    request_data: &mut RequestData,
    mode: Mode,
) -> Result<(), PresidioResponseError> {
    let tokens = pii_tokens(request_data)?;
    if tokens.is_empty() && mode == Mode::Unmask {
        tracing::debug!("Presidio response unmask found no pii_tokens in request metadata");
    }
    let presidio_config = presidio_settings_from_request(request_data)?;
    let mut masking = Masking {
        context,
        request_data,
        tokens: &tokens,
        presidio_config: presidio_config.as_ref(),
        mode,
    };
    let Some(choices) = response.choices.as_mut() else {
        return Ok(());
    };
    for choice in choices {
        let Some(message) = choice.message.as_mut() else {
            continue;
        };
        if let Some(content) = message.content.as_mut() {
            masking.content(content).await?;
        }
        masking.tools(message).await?;
    }
    Ok(())
}

struct Masking<'mask> {
    context: HookContext<'mask>,
    request_data: &'mask mut RequestData,
    tokens: &'mask IndexMap<String, String>,
    presidio_config: Option<&'mask PresidioPerRequestConfig>,
    mode: Mode,
}

impl Masking<'_> {
    async fn apply(&mut self, text: &str) -> Result<String, PresidioResponseError> {
        if self.mode == Mode::Unmask {
            return Ok(unmask_pii_text(text, self.tokens));
        }
        let context = self.context;
        let call = CheckPiiCall {
            text,
            output_parse_pii: false,
            presidio_config: self.presidio_config,
            dynamic_params: context.dynamic_params,
            pii_entities_config: context.pii_entities_config,
            request_data: &mut *self.request_data,
        };
        context
            .anonymizer
            .check_pii(context.analyzer, call)
            .await
            .map_err(PresidioResponseError::from)
    }

    async fn content(&mut self, content: &mut Content) -> Result<(), PresidioResponseError> {
        let Content::Text(text) = content else {
            if let Content::Blocks(items) = content {
                self.blocks(items).await?;
            }
            return Ok(());
        };
        *text = self.apply(text).await?;
        Ok(())
    }

    async fn blocks(&mut self, items: &mut [ContentItem]) -> Result<(), PresidioResponseError> {
        for item in items {
            if let ContentItem::Object(part) = item
                && let Some(text) = part.text.clone()
            {
                let masked = self.apply(&text).await?;
                part.set_text(masked);
            }
        }
        Ok(())
    }

    async fn tools(
        &mut self,
        message: &mut ChatResponseMessage,
    ) -> Result<(), PresidioResponseError> {
        if let Some(calls) = message.tool_calls.as_mut() {
            self.calls(calls).await?;
        }
        if let Some(function_call) = message.function_call.as_mut() {
            self.arguments(&mut function_call.arguments, &mut function_call.extra)
                .await?;
        }
        Ok(())
    }

    async fn calls(&mut self, calls: &mut [ToolCallItem]) -> Result<(), PresidioResponseError> {
        for item in calls {
            let ToolCallItem::Object(call) = item else {
                continue;
            };
            let Some(function) = call.function.as_mut() else {
                continue;
            };
            self.arguments(&mut function.arguments, &mut function.extra)
                .await?;
        }
        Ok(())
    }

    async fn arguments(
        &mut self,
        arguments: &mut Option<String>,
        extra: &mut Extensions,
    ) -> Result<(), PresidioResponseError> {
        let Some(text) = arguments.clone() else {
            return Ok(());
        };
        let masked = self.apply(&text).await?;
        extra.remove("arguments");
        *arguments = Some(masked);
        Ok(())
    }
}
