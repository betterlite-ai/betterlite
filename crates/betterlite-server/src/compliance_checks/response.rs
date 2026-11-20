use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ComplianceCheckResult {
    pub check_name: &'static str,
    pub article: &'static str,
    pub passed: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ComplianceResponse {
    pub compliant: bool,
    pub regulation: &'static str,
    pub checks: Vec<ComplianceCheckResult>,
}

impl ComplianceResponse {
    pub(crate) fn from_checks(
        regulation: &'static str,
        checks: Vec<ComplianceCheckResult>,
    ) -> Self {
        let compliant = checks.iter().all(|check| check.passed);
        Self {
            compliant,
            regulation,
            checks,
        }
    }
}
