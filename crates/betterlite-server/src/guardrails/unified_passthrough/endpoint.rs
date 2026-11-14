use std::collections::BTreeMap;
use std::marker::PhantomData;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::passthrough_guardrail::PassThroughEndpointHandler;
use betterlite_core::passthrough_guardrail::errors::PassthroughGuardrailError;
use betterlite_core::passthrough_guardrail::settings::{
    PassThroughGuardrailSettings, PassthroughGuardrailInputs, PassthroughInputType, RequestData,
    ResponseCall,
};
use betterlite_core::passthrough_guardrail::traits::{
    PassthroughGuardrail, PassthroughLogging, PassthroughPolicy,
};
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_translation::{
    BaseTranslation, TranslationCall, transform_user_key_metadata,
};
use betterlite_guardrails::passthrough_policy;
use betterlite_models::decisions::Json;

// ast-grep-ignore: no-long-names
pub struct PassthroughEndpointTranslation<G> {
    pub guardrail: PhantomData<fn() -> G>,
}

// ast-grep-ignore: no-long-names
impl<G: GuardrailContract> Default for PassthroughEndpointTranslation<G> {
    fn default() -> Self {
        Self {
            guardrail: PhantomData,
        }
    }
}

// ast-grep-ignore: no-long-names
impl<G: GuardrailContract> BaseTranslation for PassthroughEndpointTranslation<G> {
    type Guardrail = G;
    type LogTarget = GuardrailLogTarget;
    type Failure = PassthroughGuardrailError<GuardrailFailure>;

    const DELIVERS_ENDED_STREAM_REWRITES: bool = false;
    const ASSEMBLES_STREAMED_RESPONSE: bool = false;

    async fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> Result<(), Self::Failure> {
        let bridge = PassthroughGuardrailBridge { guardrail };
        let mut target = logging_obj.map(|logging| PassthroughLogTarget { logging });
        PassThroughEndpointHandler::process_input_messages(
            data,
            &bridge,
            target.as_mut(),
            &PassthroughPolicyBridge,
        )
        .await
    }

    async fn process_output_response(
        &self,
        response: GuardrailValue,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> Result<GuardrailValue, Self::Failure> {
        let bridge = PassthroughGuardrailBridge { guardrail };
        let mut target = call
            .logging_obj
            .map(|logging| PassthroughLogTarget { logging });
        PassThroughEndpointHandler::process_output_response(
            response,
            &bridge,
            ResponseCall {
                logging_obj: target.as_mut(),
                user_api_key_dict: call.user_api_key_dict,
                request_data: call.request_data,
            },
            &PassthroughPolicyBridge,
        )
        .await
    }
}

// G cannot get a PassthroughGuardrail impl from here, so the bridge carries it to B3573's handler
struct PassthroughGuardrailBridge<'state, G> {
    guardrail: &'state G,
}

impl<'state, G: GuardrailContract> PassthroughGuardrail for PassthroughGuardrailBridge<'state, G> {
    type Failure = GuardrailFailure;
    type LogTarget = PassthroughLogTarget<'state>;

    fn guardrail_name(&self) -> Option<&str> {
        None
    }

    async fn apply_guardrail(
        &self,
        inputs: PassthroughGuardrailInputs,
        request_data: &mut RequestData,
        input_type: PassthroughInputType,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> Result<PassthroughGuardrailInputs, Self::Failure> {
        let forwarded = GuardrailApiInputs {
            texts: Some(inputs.texts),
            images: None,
            tools: None,
            tool_calls: None,
            structured_messages: None,
            model: inputs.model,
            stream_holdback_chars: None,
        };
        let logging = logging_obj.map(|target| &mut *target.logging);
        let answer = self
            .guardrail
            .apply_guardrail(
                forwarded,
                request_data,
                match input_type {
                    PassthroughInputType::Request => GuardrailInputType::Request,
                    PassthroughInputType::Response => GuardrailInputType::Response,
                },
                logging,
            )
            .await?;
        Ok(PassthroughGuardrailInputs {
            texts: answer.texts.unwrap_or_default(),
            model: answer.model,
        })
    }
}

// the unified path's log target has no passthrough config, so B3573 skips field targeting
struct PassthroughLogTarget<'logging> {
    logging: &'logging mut GuardrailLogTarget,
}

impl PassthroughLogging for PassthroughLogTarget<'_> {
    fn passthrough_guardrails_config(&self) -> Option<&Json> {
        None
    }
}

// B3601's policy lives in betterlite-guardrails; B3573 takes it through betterlite-core's trait
struct PassthroughPolicyBridge;

impl PassthroughPolicy for PassthroughPolicyBridge {
    fn get_settings(
        &self,
        config: &Json,
        guardrail_name: &str,
    ) -> Option<PassThroughGuardrailSettings> {
        let found = passthrough_policy::get_settings(Some(config), guardrail_name)?;
        Some(PassThroughGuardrailSettings {
            request_fields: found.request_fields,
            response_fields: found.response_fields,
        })
    }

    fn extract_fields(&self, data: &Json, expressions: &[String]) -> String {
        passthrough_policy::extract_fields(data, expressions)
    }

    fn user_key_metadata(&self, user_api_key_dict: Option<&RequestData>) -> RequestData {
        transform_user_key_metadata(user_api_key_dict)
    }
}

mod tests;
