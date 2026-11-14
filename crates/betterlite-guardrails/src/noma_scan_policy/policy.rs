use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde_json::value::{RawValue, to_raw_value};

use super::error::NomaError;
use super::messages::{
    message_content, replace_last_user_message, replace_response_content, response_id, scan_items,
};
use super::records::{GuardrailRun, ScanTiming, record_scan};
use super::transport::{ScanTarget, call_noma_api};
use super::verdict::{Verdict, decide_verdict, determine_guardrail_status};
use super::wire::{InputMessage, TextPart};
use super::{MessageRole, NomaScanPolicy, ScanCallContext, ScanOutcome};

enum Subject<'request> {
    UserMessage {
        scanned: &'request str,
    },
    LlmResponse {
        request_id: Option<&'request str>,
        content: &'request str,
    },
}

// One scan: call Noma, write the standard logging record, then decide what the answer means
async fn scan_and_record(
    policy: &NomaScanPolicy,
    request_data: &mut BTreeMap<String, GuardrailValue>,
    call: ScanCallContext<'_>,
    items: &[Box<RawValue>],
    subject: Subject<'_>,
) -> Result<ScanOutcome, NomaError> {
    let started = (policy.now)();
    let (role, llm_request_id, scanned) = match subject {
        Subject::UserMessage { scanned } => (MessageRole::User, None, scanned),
        Subject::LlmResponse {
            request_id,
            content,
        } => (MessageRole::Assistant, request_id, content),
    };
    let target = ScanTarget {
        user_auth: call.user_auth,
        extra_data: call.extra_data,
        llm_request_id,
    };
    let response = call_noma_api(policy, items, request_data, &target).await?;
    let ended = (policy.now)();
    let status = determine_guardrail_status(&response);
    let run = GuardrailRun {
        response: &response,
        status,
        event_type: call.event_type,
        timing: ScanTiming { started, ended },
    };
    record_scan(policy, request_data, &run)?;
    let verdict = decide_verdict(&policy.settings, role, scanned, &response)?;
    let anonymized = match verdict {
        Verdict::Allowed => None,
        Verdict::Anonymized(text) => Some(text),
    };
    Ok(ScanOutcome {
        status,
        response,
        anonymized,
    })
}

impl NomaScanPolicy {
    pub async fn process_user_message_check(
        &self,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        call: ScanCallContext<'_>,
    ) -> Result<Option<ScanOutcome>, NomaError> {
        let Some(items) = scan_items(request_data)? else {
            return Ok(None);
        };
        let scanned =
            serde_json::to_string(&items).map_err(|source| NomaError::PayloadJson { source })?;
        let outcome = scan_and_record(
            self,
            request_data,
            call,
            &items,
            Subject::UserMessage { scanned: &scanned },
        )
        .await?;
        if let Some(text) = &outcome.anonymized {
            replace_last_user_message(request_data, text)?;
            tracing::debug!("Noma guardrail anonymized user message: {text}");
        }
        Ok(Some(outcome))
    }

    pub async fn process_llm_response_check(
        &self,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        response: &mut GuardrailValue,
        call: ScanCallContext<'_>,
    ) -> Result<Option<ScanOutcome>, NomaError> {
        let request_id = response_id(response)?.map(str::to_owned);
        let Some(content) = message_content(response)?.map(str::to_owned) else {
            return Ok(None);
        };
        let item = InputMessage {
            kind: "message",
            role: "assistant",
            content: [TextPart {
                kind: "input_text",
                text: &content,
            }],
        };
        let items = [to_raw_value(&item).map_err(|source| NomaError::PayloadJson { source })?];
        let subject = Subject::LlmResponse {
            request_id: request_id.as_deref(),
            content: &content,
        };
        let outcome = scan_and_record(self, request_data, call, &items, subject).await?;
        if let Some(text) = &outcome.anonymized {
            replace_response_content(response, text);
            tracing::debug!("Noma guardrail anonymized LLM response: {text}");
        }
        Ok(Some(outcome))
    }
}
