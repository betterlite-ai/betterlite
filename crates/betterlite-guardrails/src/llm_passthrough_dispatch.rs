use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;

use crate::guardrail_translation::{BaseTranslation, TranslationCall};

pub struct LlmPassthroughRouteHandler<H: BaseTranslation> {
    providers: BTreeMap<String, H>,
}

impl<H: BaseTranslation> LlmPassthroughRouteHandler<H> {
    // a later entry with the same name replaces an earlier one, the way dict.update leaves LiteLLM's table
    #[must_use]
    pub fn new(providers: Vec<(String, H)>) -> Self {
        let mut table = BTreeMap::new();
        for (name, handler) in providers {
            table.insert(name, handler);
        }
        Self { providers: table }
    }
}

// the trait hands back a Send future, so the handler, its guardrail and its log target must be Send too
impl<H> BaseTranslation for LlmPassthroughRouteHandler<H>
where
    H: BaseTranslation + Sync,
    H::Guardrail: Sync,
    H::LogTarget: Send,
{
    type Guardrail = H::Guardrail;
    type LogTarget = H::LogTarget;
    type Failure = H::Failure;

    async fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> Result<(), Self::Failure> {
        let found = provider_name(data).and_then(|name| self.providers.get(name));
        let Some(handler) = found else {
            log_no_handler(data.get("custom_llm_provider"));
            return Ok(());
        };
        handler
            .process_input_messages(data, guardrail, logging_obj)
            .await
    }

    async fn process_output_response(
        &self,
        response: GuardrailValue,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> Result<GuardrailValue, Self::Failure> {
        let TranslationCall {
            user_api_key_dict,
            request_data,
            logging_obj,
            stream_transform_sink,
            deliver_ended_stream_rewrites,
        } = call;
        let found = request_data
            .as_deref()
            .and_then(provider_name)
            .and_then(|name| self.providers.get(name));
        let Some(handler) = found else {
            log_no_handler(
                request_data
                    .as_deref()
                    .and_then(|fields| fields.get("custom_llm_provider")),
            );
            return Ok(response);
        };
        let forwarded = TranslationCall {
            user_api_key_dict,
            request_data,
            logging_obj,
            stream_transform_sink,
            deliver_ended_stream_rewrites,
        };
        handler
            .process_output_response(response, guardrail, forwarded)
            .await
    }
}

fn provider_name(data: &BTreeMap<String, GuardrailValue>) -> Option<&str> {
    let Some(GuardrailValue::Text(name)) = data.get("custom_llm_provider") else {
        return None;
    };
    (!name.is_empty()).then_some(name.as_str())
}

fn log_no_handler(provider: Option<&GuardrailValue>) {
    let label = match provider {
        None | Some(GuardrailValue::Null) => "None".to_owned(),
        Some(GuardrailValue::Text(text)) => text.clone(),
        Some(value) => serde_json::to_string(value).unwrap_or_default(),
    };
    tracing::debug!(
        "LlmPassthroughRouteHandler: no handler for provider={label}, skipping guardrail"
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassthroughTranslationKind {
    Endpoint,
    LlmRoute,
}

// LiteLLM's guardrail_translation_mappings; the sync llm_passthrough_route route has no row on purpose
pub const PASSTHROUGH_TRANSLATION_KINDS: [(CallType, PassthroughTranslationKind); 2] = [
    (
        CallType::PassThroughEndpoint,
        PassthroughTranslationKind::Endpoint,
    ),
    (
        CallType::AllmPassthroughRoute,
        PassthroughTranslationKind::LlmRoute,
    ),
];

mod tests;
