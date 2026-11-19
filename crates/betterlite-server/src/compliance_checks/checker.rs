use super::model::{ComplianceRequest, GuardrailEvidence, GuardrailStatus};
use super::response::{ComplianceCheckResult, ComplianceResponse};

const PRE_CALL: &str = "pre_call";
const USER_ID: &str = "user_id";
const MODEL: &str = "model";
const TIMESTAMP: &str = "timestamp";
const GUARDRAIL_RESULTS: &str = "guardrail_results";

#[derive(Clone, Copy)]
pub enum Regulation {
    EuAiAct,
    Gdpr,
}

impl Regulation {
    pub(crate) const fn names(self) -> (&'static str, &'static str) {
        match self {
            Self::EuAiAct => ("EU AI Act", "check_eu_ai_act_compliance"),
            Self::Gdpr => ("GDPR", "check_gdpr_compliance"),
        }
    }
}

#[derive(Clone, Copy)]
struct CountedLabels {
    check_name: &'static str,
    article: &'static str,
    counted: &'static str,
    empty: &'static str,
}

const APPLIED_LABELS: CountedLabels = CountedLabels {
    check_name: "Guardrails applied",
    article: "Art. 9",
    counted: "guardrail(s) applied",
    empty: "No guardrails applied",
};

const SCREENING_LABELS: CountedLabels = CountedLabels {
    check_name: "Content screened before LLM",
    article: "Art. 5",
    counted: "pre-call guardrail(s) screened content",
    empty: "No pre-call screening applied",
};

const PROTECTION_LABELS: CountedLabels = CountedLabels {
    check_name: "Data protection applied",
    article: "Art. 32",
    counted: "pre-call guardrail(s) protect data",
    empty: "No pre-call data protection applied",
};

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
    pub fn response(&self, regulation: Regulation) -> ComplianceResponse {
        let (name, _) = regulation.names();
        let checks = match regulation {
            Regulation::EuAiAct => vec![
                counted(self.guardrails.len(), APPLIED_LABELS),
                counted(self.by_mode(PRE_CALL).len(), SCREENING_LABELS),
                self.audit("Art. 12"),
            ],
            Regulation::Gdpr => vec![
                counted(self.by_mode(PRE_CALL).len(), PROTECTION_LABELS),
                self.sensitive(),
                self.audit("Art. 30"),
            ],
        };
        ComplianceResponse::from_checks(name, checks)
    }

    fn by_mode(&self, mode: &str) -> Vec<&'request GuardrailEvidence> {
        let mut matched = Vec::new();
        for evidence in &self.guardrails {
            if evidence.mode.matches(mode) {
                matched.push(*evidence);
            }
        }
        matched
    }

    fn sensitive(&self) -> ComplianceCheckResult {
        let pre_call = self.by_mode(PRE_CALL);
        let intervened = pre_call.iter().any(|evidence| {
            matches!(
                evidence.status,
                GuardrailStatus::Intervened | GuardrailStatus::Failed | GuardrailStatus::Blocked
            )
        });
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

    fn audit(&self, article: &'static str) -> ComplianceCheckResult {
        let mut missing = Vec::new();
        if self.request.user_id.as_deref().is_none_or(str::is_empty) {
            missing.push(USER_ID);
        }
        if self.request.model.as_deref().is_none_or(str::is_empty) {
            missing.push(MODEL);
        }
        if self.request.timestamp.as_deref().is_none_or(str::is_empty) {
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

fn counted(matched: usize, labels: CountedLabels) -> ComplianceCheckResult {
    let (passed, detail) = if matched == 0 {
        (false, labels.empty.to_owned())
    } else {
        (true, format!("{matched} {}", labels.counted))
    };
    ComplianceCheckResult {
        check_name: labels.check_name,
        article: labels.article,
        passed,
        detail,
    }
}
