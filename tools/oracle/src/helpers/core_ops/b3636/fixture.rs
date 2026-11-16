use std::sync::Arc;

use betterlite_core::openai_streaming_assembly::RelayTokenCounter;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use betterlite_streaming::stream_usage_assembly::wire::Usage;
use betterlite_streaming::streamed_response_assembly::cost::{
    CostError, CostInput, StreamCostCalculator,
};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::{Message, MessageContent, Part};
use rust_decimal::Decimal;
use serde::Deserialize;

use super::failure;

const IMAGE_URL_TYPE: &str = "image_url";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct FixtureCounter {
    messages: i64,
    response_text: i64,
}

impl RelayTokenCounter for FixtureCounter {
    fn count_messages(&self, _model: &str, messages: &[Message]) -> Result<i64, CountError> {
        for message in messages {
            let Some(MessageContent::Parts(parts)) = &message.content else {
                continue;
            };
            for part in parts {
                let Part::Block(block) = part else {
                    continue;
                };
                if block.kind == IMAGE_URL_TYPE {
                    return Err(CountError::UnsupportedContent {
                        kind: block.kind.clone(),
                    });
                }
            }
        }
        Ok(self.messages)
    }

    fn count_text(&self, _text: &str) -> Result<i64, CountError> {
        Ok(self.response_text)
    }

    fn count_response_text(&self, _model: &str, _text: &str) -> Result<i64, CountError> {
        Ok(self.response_text)
    }
}

pub(super) struct NoCosts;

impl StreamCostCalculator for NoCosts {
    fn completion_cost(
        &self,
        _input: &CostInput<'_>,
        _provider: Option<&str>,
    ) -> Result<Option<Decimal>, CostError> {
        Ok(None)
    }

    fn prompt_completion_cost(
        &self,
        _model: &str,
        _usage: &Usage,
    ) -> Result<(Decimal, Decimal), CostError> {
        Err(CostError {
            message: "a relayed stream carries no price".to_owned(),
        })
    }
}

#[derive(Default)]
pub(super) struct SilentHooks;

impl LoggerSink for SilentHooks {
    fn debug(&self, _payload: &Payload) -> bool {
        true
    }

    fn info(&self, _payload: &Payload) -> bool {
        true
    }

    fn error(&self, _payload: &Payload) -> bool {
        true
    }

    fn debug_enabled(&self) -> bool {
        false
    }
}

impl ConsoleSink for SilentHooks {
    fn print(&self, _payload: &Payload) -> bool {
        false
    }
}

pub(super) fn hooks() -> Result<VerboseHooks<SilentHooks, SilentHooks>, Box<HelperError>> {
    let redactor =
        Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| failure(error.to_string()))?;
    Ok(VerboseHooks {
        logger: SilentHooks,
        console: SilentHooks,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(redactor),
    })
}
