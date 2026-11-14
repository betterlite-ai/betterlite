use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde_json::value::{RawValue, to_raw_value};

use crate::guardrail_records::record::GuardrailStatus;

use super::error::NomaError;
use super::messages::{
    message_content, replace_last_user_message, replace_response_content, response_id, scan_items,
};
use super::records::{GuardrailRun, ScanTiming, record_scan};
use super::transport::{ScanTarget, call_noma_api};
use super::verdict::{Verdict, decide_verdict, determine_guardrail_status};
use super::wire::{InputMessage, ScanResponse, TextPart};
use super::{MessageRole, NomaScanPolicy, ScanCallContext, ScanOutcome};

struct ScanRound<'round> {
    started: SystemTime,
    role: MessageRole,
    items: &'round [Box<RawValue>],
    scanned: &'round str,
    llm_request_id: Option<&'round str>,
}

// One scan: call Noma, write the standard logging record, then decide what the answer means
async fn scan_and_record(
    policy: &NomaScanPolicy,
    request_data: &mut BTreeMap<String, GuardrailValue>,
    call: ScanCallContext<'_>,
    round: &ScanRound<'_>,
) -> Result<(ScanResponse, GuardrailStatus, Verdict), NomaError> {
    let target = ScanTarget {
        user_auth: call.user_auth,
        extra_data: call.extra_data,
        llm_request_id: round.llm_request_id,
    };
    let response = call_noma_api(policy, round.items, request_data, &target).await?;
    let ended = (policy.now)();
    let status = determine_guardrail_status(&response);
    let run = GuardrailRun {
        response: &response,
        status,
        event_type: call.event_type,
        timing: ScanTiming {
            started: round.started,
            ended,
        },
    };
    record_scan(policy, request_data, &run)?;
    let verdict = decide_verdict(&policy.settings, round.role, round.scanned, &response)?;
    Ok((response, status, verdict))
}

impl NomaScanPolicy {
    pub async fn process_user_message_check(
        &self,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        call: ScanCallContext<'_>,
    ) -> Result<Option<ScanOutcome>, NomaError> {
        let started = (self.now)();
        let Some(items) = scan_items(request_data)? else {
            return Ok(None);
        };
        let scanned =
            serde_json::to_string(&items).map_err(|source| NomaError::PayloadJson { source })?;
        let round = ScanRound {
            started,
            role: MessageRole::User,
            items: &items,
            scanned: &scanned,
            llm_request_id: None,
        };
        let (response, status, verdict) = scan_and_record(self, request_data, call, &round).await?;
        let anonymized = match verdict {
            Verdict::Allowed => None,
            Verdict::Anonymized(text) => {
                replace_last_user_message(request_data, &text)?;
                tracing::debug!("Noma guardrail anonymized user message: {text}");
                Some(text)
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
        let round = ScanRound {
            started,
            role: MessageRole::Assistant,
            items: &items,
            scanned: &content,
            llm_request_id: request_id.as_deref(),
        };
        let (scan_response, status, verdict) =
            scan_and_record(self, request_data, call, &round).await?;
        let anonymized = match verdict {
            Verdict::Allowed => None,
            Verdict::Anonymized(text) => {
                replace_response_content(response, &text);
                tracing::debug!("Noma guardrail anonymized LLM response: {text}");
                Some(text)
            }
        };
        Ok(Some(ScanOutcome {
            status,
            response: scan_response,
            anonymized,
        }))
    }
}
