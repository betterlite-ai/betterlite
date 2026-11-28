#![cfg(test)]

use std::panic::Location;
use std::path::PathBuf;

use rstest::rstest;

use betterlite_reporting::bug_report::notice::{NOTICE_PREFIX, bug_report_notice};
use betterlite_reporting::bug_report::record::{Deployment, Surface};
use betterlite_reporting::safe_proxy_config::{ProxyCall, ProxyConfigSnapshot};

use super::BugReportNotices;
use crate::api::proxy_endpoint_support::{FailureFacts, ProxyDiagnostics};

const CONFIG_LINES: [&str; 2] = [
    "general_settings.store_model_in_db = true",
    "litellm_settings.success_callback = [openai]",
];

fn snapshot() -> ProxyConfigSnapshot {
    serde_json::from_str(
        r#"{"config": {"litellm_settings": {"success_callback": ["openai"]}}, "general_settings": {"store_model_in_db": true}}"#,
    )
    .unwrap()
}

fn missing_docker() -> PathBuf {
    PathBuf::from("/missing/dockerenv")
}

fn notices(enabled: bool, dockerenv: PathBuf) -> BugReportNotices {
    BugReportNotices {
        enabled,
        snapshot: snapshot(),
        dockerenv,
    }
}

#[track_caller]
fn failure(status_code: Option<u16>) -> FailureFacts<'static> {
    FailureFacts {
        message: "boom",
        exception_type: "FailureError",
        status_code,
        param: None,
        database_error: None,
        location: Location::caller(),
    }
}

fn call() -> ProxyCall {
    ProxyCall {
        call_type: Some(String::from("acompletion")),
        custom_llm_provider: Some(String::from("openai")),
        stream: Some(true),
    }
}

#[rstest]
#[case::too_many_requests(Some(429))]
#[case::internal_server_error(Some(500))]
#[case::bad_request(Some(400))]
fn report_answers_none_when_the_failure_carries_a_status(#[case] status_code: Option<u16>) {
    let notices = notices(true, missing_docker());
    assert_eq!(notices.report(&failure(status_code), call()), None);
}

#[test]
fn report_answers_none_when_the_link_is_disabled() {
    let notices = notices(false, missing_docker());
    assert_eq!(notices.report(&failure(None), call()), None);
}

#[test]
fn report_builds_the_proxy_record_from_the_failure_and_the_snapshot() {
    let (facts, call_line) = (failure(None), line!());
    let notices = notices(true, missing_docker());
    let report = notices.report(&facts, call()).unwrap();
    assert_eq!(report.environment.surface, Surface::Proxy);
    assert_eq!(report.environment.deployment, None);
    assert_eq!(report.environment.config_lines, CONFIG_LINES);
    assert_eq!(report.exception_type, "FailureError");
    assert_eq!(report.litellm_frames, [format!("{}:{call_line}", file!())]);
    assert_eq!(
        (report.call_type, report.custom_llm_provider, report.stream),
        (
            Some(String::from("acompletion")),
            Some(String::from("openai")),
            Some(true)
        )
    );
}

#[rstest]
#[case::in_a_container(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"), Some(Deployment::Docker))]
#[case::on_a_host(missing_docker(), None)]
fn report_reads_the_deployment_from_the_dockerenv_path(
    #[case] dockerenv: PathBuf,
    #[case] deployment: Option<Deployment>,
) {
    let notices = notices(true, dockerenv);
    let report = notices
        .report(&failure(None), ProxyCall::default())
        .unwrap();
    assert_eq!(report.environment.deployment, deployment);
}

#[test]
fn the_trait_impl_answers_the_notice_with_no_call_fields() {
    let (facts, call_line) = (failure(None), line!());
    let notices = notices(true, missing_docker());
    let report = notices.report(&facts, ProxyCall::default()).unwrap();
    assert_eq!(
        (report.call_type, report.custom_llm_provider, report.stream),
        (None, None, None)
    );
    assert_eq!(report.litellm_frames, [format!("{}:{call_line}", file!())]);
    let notice = notices.bug_report_notice(&facts).unwrap();
    assert!(notice.starts_with(NOTICE_PREFIX), "{notice}");
    assert_eq!(notice, bug_report_notice(&report));
}

#[test]
fn the_trait_impl_answers_none_where_report_does() {
    let notices = notices(true, missing_docker());
    assert_eq!(notices.bug_report_notice(&failure(Some(500))), None);
    let disabled = notices(false, missing_docker());
    assert_eq!(disabled.bug_report_notice(&failure(None)), None);
}
