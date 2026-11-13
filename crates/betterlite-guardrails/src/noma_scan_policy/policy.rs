use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde_json::value::to_raw_value;

use super::error::NomaError;
use super::messages::{
    converted_input, message_content, messages_from_request, replace_last_user_message,
    replace_response_content, response_id,
};
use super::records::{GuardrailRun, ScanTiming};
use super::transport::ScanTarget;
use super::verdict::Verdict;
use super::wire::{InputMessage, TextPart};
use super::{MessageRole, NomaScanPolicy, ScanCallContext, ScanOutcome};

impl NomaScanPolicy {
    pub async fn process_user_message_check(
        &self,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        call: ScanCallContext<'_>,
    ) -> Result<Option<ScanOutcome>, NomaError> {
        let started = (self.now)();
        let Some(messages) = messages_from_request(request_data)? else {
            return Ok(None);
        };
        let converted = converted_input(&messages)?;
        let mut items = converted.items;
        if let Some(instructions) = converted
            .instructions
            .as_deref()
            .filter(|text| !text.is_empty())
        {
            let system = InputMessage {
                kind: "message",
                role: "system",
                content: [TextPart {
                    kind: "input_text",
                    text: instructions,
                }],
            };
            let raw = to_raw_value(&system).map_err(|source| NomaError::PayloadJson { source })?;
            items.insert(0, raw);
        }
        if items.is_empty() {
            return Ok(None);
        }
        let target = ScanTarget {
            user_auth: call.user_auth,
            extra_data: call.extra_data,
            llm_request_id: None,
        };
        let response = self.call_noma_api(&items, request_data, &target).await?;
        let ended = (self.now)();
        let status = Self::determine_guardrail_status(&response);
        let timing = ScanTiming { started, ended };
        self.record_scan(
            request_data,
            &GuardrailRun {
                response: &response,
                status,
                event_type: call.event_type,
                timing,
            },
        )?;
        let scanned =
            serde_json::to_string(&items).map_err(|source| NomaError::PayloadJson { source })?;
        let verdict = self.decide_verdict(MessageRole::User, &scanned, &response)?;
        let anonymized = match verdict {
            Verdict::Allowed => None,
            Verdict::Anonymized(anonymized) => {
                replace_last_user_message(request_data, &anonymized)?;
                tracing::debug!("Noma guardrail anonymized user message: {anonymized}");
                Some(anonymized)
            }
        };
        Ok(Some(ScanOutcome {
            status,
            response,
            anonymized,
        }))
    }

    pub async fn process_llm_response_check(
        &self,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        response: &mut GuardrailValue,
        call: ScanCallContext<'_>,
    ) -> Result<Option<ScanOutcome>, NomaError> {
        let started = (self.now)();
        let request_id = response_id(response)?.map(str::to_owned);
        let Some(scanned_text) = message_content(response)? else {
            return Ok(None);
        };
        let content = scanned_text.to_owned();
        let item = InputMessage {
            kind: "message",
            role: "assistant",
            content: [TextPart {
                kind: "input_text",
                text: &content,
            }],
        };
        let raw = to_raw_value(&item).map_err(|source| NomaError::PayloadJson { source })?;
        let target = ScanTarget {
            user_auth: call.user_auth,
            extra_data: call.extra_data,
            llm_request_id: request_id.as_deref(),
        };
        let items = [raw];
        let scan_response = self.call_noma_api(&items, request_data, &target).await?;
        let ended = (self.now)();
        let status = Self::determine_guardrail_status(&scan_response);
        let timing = ScanTiming { started, ended };
        self.record_scan(
            request_data,
            &GuardrailRun {
                response: &scan_response,
                status,
                event_type: call.event_type,
                timing,
            },
        )?;
        let verdict = self.decide_verdict(MessageRole::Assistant, &content, &scan_response)?;
        let anonymized = match verdict {
            Verdict::Allowed => None,
            Verdict::Anonymized(anonymized) => {
                replace_response_content(response, &anonymized);
                tracing::debug!("Noma guardrail anonymized LLM response: {anonymized}");
                Some(anonymized)
            }
        };
        Ok(Some(ScanOutcome {
            status,
            response: scan_response,
            anonymized,
        }))
    }
}
