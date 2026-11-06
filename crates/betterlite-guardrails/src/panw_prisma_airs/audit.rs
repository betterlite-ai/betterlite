use std::time::{Duration, SystemTime};

use serde::Serialize;

use super::verdict::VerdictSummary;

// Mirrors the GuardrailEventHooks members PANW scans run at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    PreCall,
    PostCall,
}

// LiteLLM passes is_response next to a pre_call or post_call event type, and the two always agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Request,
    Response,
}

impl Direction {
    #[must_use]
    pub const fn stage(self) -> Stage {
        match self {
            Self::Request => Stage::PreCall,
            Self::Response => Stage::PostCall,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailStatus {
    Success,
    GuardrailIntervened,
    GuardrailFailedToRespond,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScanIdEntry {
    pub guardrail: String,
    pub stage: Stage,
    pub provider: String,
    pub scan_id: String,
}

// Mirrors StandardLoggingGuardrailInformation for the fields PANW fills in.
#[derive(Debug, Clone)]
pub struct GuardrailInformation {
    pub provider: String,
    pub status: GuardrailStatus,
    pub response: VerdictSummary,
    pub start_time: SystemTime,
    pub end_time: SystemTime,
    pub duration: Duration,
    pub event_type: Stage,
}

// What LiteLLM writes into the request metadata bucket; the hooks drain it into the real request.
#[derive(Debug, Default)]
pub struct GuardrailAudit {
    pub scan_ids: Vec<String>,
    pub scan_metadata: Vec<ScanIdEntry>,
    pub applied_guardrails: Vec<String>,
    pub information: Vec<GuardrailInformation>,
}

impl GuardrailAudit {
    // Mirrors LiteLLM's fn add_guardrail_scan_id.
    pub fn add_scan_id(
        &mut self,
        scan_id: Option<&str>,
        guardrail: &str,
        provider: &str,
        stage: Stage,
    ) {
        let Some(id) = scan_id.filter(|found| !found.is_empty()) else {
            return;
        };
        if !self.scan_ids.iter().any(|existing| existing == id) {
            self.scan_ids.push(id.to_owned());
        }
        let entry = ScanIdEntry {
            guardrail: guardrail.to_owned(),
            stage,
            provider: provider.to_owned(),
            scan_id: id.to_owned(),
        };
        if !self.scan_metadata.contains(&entry) {
            self.scan_metadata.push(entry);
        }
    }

    // Mirrors LiteLLM's fn add_guardrail_to_applied_guardrails_header.
    pub fn add_applied_guardrail(&mut self, guardrail: &str) {
        if !self.applied_guardrails.iter().any(|name| name == guardrail) {
            self.applied_guardrails.push(guardrail.to_owned());
        }
    }
}
