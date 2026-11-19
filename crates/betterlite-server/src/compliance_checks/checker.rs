use super::model::{ComplianceRequest, GuardrailEvidence, GuardrailStatus};
use super::response::{ComplianceCheckResult, ComplianceResponse};

pub const EU_AI_ACT: &str = "EU AI Act";
pub const GDPR: &str = "GDPR";
const PRE_CALL: &str = "pre_call";
const USER_ID: &str = "user_id";
const MODEL: &str = "model";
const TIMESTAMP: &str = "timestamp";
const GUARDRAIL_RESULTS: &str = "guardrail_results";

pub struct ComplianceChecker<'request> {
    request: &'request ComplianceRequest,
    guardrails: Vec<&'request GuardrailEvidence>,
}

impl<'request> ComplianceChecker<'request> {
    #[must_use]
    pub fn new(request: &'request ComplianceRequest) -> Self {
        let guardrails = request
            .guardrail_information
            .iter()
            .filter(|evidence| evidence.status != GuardrailStatus::NotRun)
            .collect();
        Self {
            request,
            guardrails,
        }
    }

    #[must_use]
    pub fn check_eu_ai_act(&self) -> Vec<ComplianceCheckResult> {
        vec![self.art_9(), self.art_5(), self.art_12()]
    }

    #[must_use]
    pub fn check_gdpr(&self) -> Vec<ComplianceCheckResult> {
        vec![self.art_32(), self.art_5_1c(), self.art_30()]
    }

    #[must_use]
    pub fn eu_ai_act_response(&self) -> ComplianceResponse {
        ComplianceResponse::from_checks(EU_AI_ACT, self.check_eu_ai_act())
    }

    #[must_use]
    pub fn gdpr_response(&self) -> ComplianceResponse {
        ComplianceResponse::from_checks(GDPR, self.check_gdpr())
    }

    fn by_mode(&self, mode: &str) -> Vec<&'request GuardrailEvidence> {
        self.guardrails
            .iter()
            .copied()
            .filter(|evidence| evidence.mode.matches(mode))
            .collect()
    }

    fn art_9(&self) -> ComplianceCheckResult {
        let applied = self.guardrails.len();
        let passed = applied > 0;
        let detail = if passed {
            format!("{applied} guardrail(s) applied")
        } else {
            "No guardrails applied".to_owned()
        };
        ComplianceCheckResult {
            check_name: "Guardrails applied",
            article: "Art. 9",
            passed,
            detail,
        }
    }

    fn art_5(&self) -> ComplianceCheckResult {
        let screened = self.by_mode(PRE_CALL).len();
        let passed = screened > 0;
        let detail = if passed {
            format!("{screened} pre-call guardrail(s) screened content")
        } else {
            "No pre-call screening applied".to_owned()
        };
        ComplianceCheckResult {
            check_name: "Content screened before LLM",
            article: "Art. 5",
            passed,
            detail,
        }
    }

    fn art_12(&self) -> ComplianceCheckResult {
        self.audit("Art. 12")
    }

    fn art_32(&self) -> ComplianceCheckResult {
        let protected = self.by_mode(PRE_CALL).len();
        let passed = protected > 0;
        let detail = if passed {
            format!("{protected} pre-call guardrail(s) protect data")
        } else {
            "No pre-call data protection applied".to_owned()
        };
        ComplianceCheckResult {
            check_name: "Data protection applied",
            article: "Art. 32",
            passed,
            detail,
        }
    }

    fn art_5_1c(&self) -> ComplianceCheckResult {
        let pre_call = self.by_mode(PRE_CALL);
        let intervened = pre_call
            .iter()
            .any(|evidence| evidence.status.is_intervention());
        let all_passed = !pre_call.is_empty()
            && pre_call
                .iter()
                .all(|evidence| evidence.status == GuardrailStatus::Success);
        let (passed, detail) = if intervened {
            (true, "Guardrail intervened to protect sensitive data")
        } else if all_passed {
            (true, "No sensitive data detected")
        } else {
            (false, "No pre-call guardrails to protect sensitive data")
        };
        ComplianceCheckResult {
            check_name: "Sensitive data protected",
            article: "Art. 5(1)(c)",
            passed,
            detail: detail.to_owned(),
        }
    }

    fn art_30(&self) -> ComplianceCheckResult {
        self.audit("Art. 30")
    }

    fn audit(&self, article: &'static str) -> ComplianceCheckResult {
        let mut missing = Vec::new();
        if absent(self.request.user_id.as_deref()) {
            missing.push(USER_ID);
        }
        if absent(self.request.model.as_deref()) {
            missing.push(MODEL);
        }
        if absent(self.request.timestamp.as_deref()) {
            missing.push(TIMESTAMP);
        }
        if self.guardrails.is_empty() {
            missing.push(GUARDRAIL_RESULTS);
        }
        let passed = missing.is_empty();
        let detail = if passed {
            "All required audit fields present".to_owned()
        } else {
            format!("Missing: {}", missing.join(", "))
        };
        ComplianceCheckResult {
            check_name: "Audit record complete",
            article,
            passed,
            detail,
        }
    }
}

// LiteLLM's bool() over an optional audit field: an empty string is missing, as an absent one is.
fn absent(value: Option<&str>) -> bool {
    value.is_none_or(str::is_empty)
}
