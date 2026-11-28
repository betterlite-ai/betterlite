use std::path::PathBuf;

use betterlite_reporting::bug_report::environment::Host;
use betterlite_reporting::bug_report::notice::bug_report_notice;
use betterlite_reporting::bug_report::record::BugReport;
use betterlite_reporting::safe_proxy_config::{
    ProxyCall, ProxyConfigSnapshot, build_proxy_bug_report,
};

use crate::api::proxy_endpoint_support::{FailureFacts, ProxyDiagnostics};

mod tests;

// the one production ProxyDiagnostics: LiteLLM's notice built from the snapshot and host the slot resolved at startup
pub struct BugReportNotices {
    pub enabled: bool,
    pub snapshot: ProxyConfigSnapshot,
    pub dockerenv: PathBuf,
}

impl BugReportNotices {
    // B4170's should_report_bug, then B10058's proxy report; the caller logs the notice, so nothing here does
    #[must_use]
    pub fn report(&self, failure: &FailureFacts<'_>, call: ProxyCall) -> Option<BugReport> {
        if !self.enabled || failure.status_code.is_some() {
            return None;
        }
        Some(build_proxy_bug_report(
            failure.exception_type,
            failure.location,
            call,
            &self.snapshot,
            Host {
                dockerenv: &self.dockerenv,
                python_version: None,
            },
        ))
    }
}

impl ProxyDiagnostics for BugReportNotices {
    fn bug_report_notice(&self, failure: &FailureFacts<'_>) -> Option<String> {
        // handle_exception_on_proxy calls build_proxy_bug_report(e) alone, so every call field stays None
        let report = self.report(failure, ProxyCall::default())?;
        Some(bug_report_notice(&report))
    }
}
