use crate::bug_report::issue_url::bug_report_issue_url;
use crate::bug_report::record::BugReport;

pub const NOTICE_PREFIX: &str = "This looks like a bug in LiteLLM rather than in your request.";

#[must_use]
pub fn strip_bug_report_notice(message: &str) -> &str {
    let Some((head, _)) = message.split_once(NOTICE_PREFIX) else {
        return message;
    };
    head.strip_suffix('\n').unwrap_or(head)
}

#[must_use]
pub fn bug_report_notice(report: &BugReport) -> String {
    let url = bug_report_issue_url(report);
    format!(
        "{NOTICE_PREFIX} File it with one click (prefilled, no request data or error text, review before submitting): {url}"
    )
}
