use std::collections::BTreeMap;
use std::marker::PhantomData;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::passthrough_guardrail::errors::PassthroughGuardrailError;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_translation::{
    BaseTranslation, TranslationCall, transform_user_key_metadata,
};
use betterlite_models::decisions::Json;
use thiserror::Error;

use crate::guardrails::bedrock_passthrough::event_stream::hook::{
    EventStreamError, PostCallSuccessHook, de_anonymize_event_stream,
};
use crate::guardrails::bedrock_passthrough::output::{self, OutputTranslationError};
use crate::guardrails::bedrock_passthrough::request::{self, RequestTranslationError};
use crate::guardrails::bedrock_passthrough::routing::Routed;
use crate::guardrails::bedrock_passthrough::text_paths::SkipFlags;

// ast-grep-ignore: no-long-names
use super::endpoint::PassthroughEndpointTranslation;

#[derive(Debug, Error)]
pub enum BedrockPassthroughError {
    #[error(transparent)]
    Request(#[from] RequestTranslationError),
    #[error(transparent)]
    Output(#[from] OutputTranslationError),
    #[error(transparent)]
    Endpoint(#[from] PassthroughGuardrailError<GuardrailFailure>),
}

// LiteLLM builds this handler per route from the guardrail's skip flags and the endpoint's settings
pub struct BedrockPassthroughTranslation<G, H> {
    pub(crate) guardrail: PhantomData<fn() -> G>,
    pub(crate) skip_system: bool,
    pub(crate) skip_tool: bool,
    pub(crate) guardrail_name: Option<String>,
    pub(crate) passthrough_guardrails_config: Option<Json>,
    pub(crate) hook: H,
}

impl<G, H> BedrockPassthroughTranslation<G, H> {
    // B3680 supplies the values from the guardrail and the endpoint; this adapter never reads config
    #[must_use]
    pub const fn new(
        hook: H,
        skip: SkipFlags,
        guardrail_name: Option<String>,
        passthrough_guardrails_config: Option<Json>,
    ) -> Self {
        Self {
            guardrail: PhantomData,
            skip_system: skip.system,
            skip_tool: skip.tool,
            guardrail_name,
            passthrough_guardrails_config,
            hook,
        }
    }
}

impl<G, H> BaseTranslation for BedrockPassthroughTranslation<G, H>
where
    G: GuardrailContract,
    H: PostCallSuccessHook + Sync,
{
    type Guardrail = G;
    type LogTarget = GuardrailLogTarget;
    type Failure = BedrockPassthroughError;

    const DELIVERS_ENDED_STREAM_REWRITES: bool = false;
    const ASSEMBLES_STREAMED_RESPONSE: bool = false;

    async fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        mut logging_obj: Option<&mut Self::LogTarget>,
    ) -> Result<(), Self::Failure> {
        let skip = SkipFlags {
            system: self.skip_system,
            tool: self.skip_tool,
        };
        let routed =
            request::process_input_messages(data, guardrail, skip, logging_obj.as_deref_mut())
                .await?;
        match routed {
            Routed::Handled => Ok(()),
            Routed::Generic => {
                // ast-grep-ignore: no-long-names
                PassthroughEndpointTranslation::<G>::new(
                    self.guardrail_name.clone(),
                    self.passthrough_guardrails_config.clone(),
                )
                .process_input_messages(data, guardrail, logging_obj)
                .await?;
                Ok(())
            }
        }
    }

    async fn process_output_response(
        &self,
        mut response: GuardrailValue,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> Result<GuardrailValue, Self::Failure> {
        let TranslationCall {
            user_api_key_dict,
            request_data,
            logging_obj: mut logging,
            stream_transform_sink,
            deliver_ended_stream_rewrites,
        } = call;
        let routed = output::process_output_response(
            &mut response,
            guardrail,
            logging.as_deref_mut(),
            transform_user_key_metadata(user_api_key_dict),
            request_data.as_deref(),
        )
        .await?;
        match routed {
            Routed::Handled => Ok(response),
            Routed::Generic => {
                let forwarded = TranslationCall {
                    user_api_key_dict,
                    request_data,
                    logging_obj: logging,
                    stream_transform_sink,
                    deliver_ended_stream_rewrites,
                };
                // ast-grep-ignore: no-long-names
                PassthroughEndpointTranslation::<G>::new(
                    self.guardrail_name.clone(),
                    self.passthrough_guardrails_config.clone(),
                )
                .process_output_response(response, guardrail, forwarded)
                .await
                .map_err(BedrockPassthroughError::Endpoint)
            }
        }
    }
}

impl<G, H> BedrockPassthroughTranslation<G, H>
where
    H: PostCallSuccessHook,
{
    // ast-grep-ignore: no-one-line-functions
    pub async fn de_anonymize_event_stream(
        &self,
        body: &[u8],
        user_api_key_dict: &BTreeMap<String, GuardrailValue>,
        data: &BTreeMap<String, GuardrailValue>,
    ) -> Result<Vec<u8>, EventStreamError<H::Error>> {
        de_anonymize_event_stream(body, &self.hook, user_api_key_dict, data).await
    }
}

mod tests;
