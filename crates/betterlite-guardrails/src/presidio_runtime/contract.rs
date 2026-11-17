use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;

use super::PresidioGuard;
use super::data::{store_request_data, to_request_data};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::guardrail_initializers::presidio::PresidioRole;
use crate::guardrail_selection::gate::GuardrailSelection;
use crate::presidio_actions::check::CheckPiiCall;
use crate::presidio_actions::request_data::{RequestData, pii_tokens};
use crate::presidio_actions::tokens::unmask_pii_text;

type Data = BTreeMap<String, GuardrailValue>;

impl GuardrailContract for PresidioGuard {
    async fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut Data,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        self.apply(inputs, request_data, input_type).await
    }
}

impl PresidioGuard {
    async fn apply(
        &self,
        inputs: GuardrailApiInputs,
        data: &mut Data,
        input_type: GuardrailInputType,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let mut inputs = inputs;
        let texts = inputs.texts.take().unwrap_or_default();
        let dynamic_params = self.base.dynamic_request_body_params(data)?;
        let mut request = to_request_data(data)?;
        let tokens = pii_tokens(&request)?;
        let masked: Result<Vec<String>, GuardrailFailure> = if self.restores(input_type, &tokens) {
            Ok(texts
                .iter()
                .map(|text| unmask_pii_text(text, &tokens))
                .collect())
        } else {
            self.check_texts(&texts, &dynamic_params, &mut request)
                .await
        };
        store_request_data(data, &request)?;
        inputs.texts = Some(masked?);
        Ok(inputs)
    }

    fn restores(&self, input_type: GuardrailInputType, tokens: &IndexMap<String, String>) -> bool {
        if input_type != GuardrailInputType::Response {
            return false;
        }
        match self.callback_role {
            Some(PresidioRole::Restore) => true,
            Some(PresidioRole::Scan) => false,
            None => !self.analyzer.settings.apply_to_output && !tokens.is_empty(),
        }
    }

    async fn check_texts(
        &self,
        texts: &[String],
        dynamic_params: &BTreeMap<String, GuardrailValue>,
        request: &mut RequestData,
    ) -> Result<Vec<String>, GuardrailFailure> {
        let mut masked = Vec::with_capacity(texts.len());
        for text in texts {
            let outcome = self
                .anonymizer
                .check_pii(
                    &self.analyzer,
                    CheckPiiCall {
                        text,
                        output_parse_pii: self.analyzer.settings.output_parse_pii,
                        presidio_config: None,
                        dynamic_params,
                        pii_entities_config: &self.pii_entities_config,
                        request_data: &mut *request,
                    },
                )
                .await?;
            masked.push(outcome);
        }
        Ok(masked)
    }
}
