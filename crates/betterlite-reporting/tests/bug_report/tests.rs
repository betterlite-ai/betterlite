use betterlite_reporting::{BugReport, EnvironmentReport, Surface, bug_report_issue_url};

#[test]
fn bug_report_records_are_serializable_and_safe() {
    let report = BugReport {
        environment: EnvironmentReport {
            surface: Surface::Sdk,
            litellm_version: String::from("1"),
            python_version: String::from("unknown"),
            deployment: None,
            config_lines: vec![String::from("debug=true")],
        },
        exception_type: String::from("Oops"),
        litellm_frames: vec![String::from("crates/betterlite/src/lib.rs:4")],
        call_type: Some(String::from("chat")),
        custom_llm_provider: Some(String::from("openai")),
        stream: Some(true),
    };
    let url = bug_report_issue_url(&report);
    assert!(url.len() < 6_000);
    assert!(!url.contains("secret error"));
    assert!(url.contains("debug%3Dtrue"));
    assert!(url.contains("%5BBug%5D%3A+Oops"));
}
