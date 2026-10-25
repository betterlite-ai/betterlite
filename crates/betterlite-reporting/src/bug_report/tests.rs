#![cfg(test)]

use std::path::Path;

use backtrace::Backtrace;
use rstest::rstest;

use crate::bug_report::eligibility::{bug_report_enabled, should_report_bug};
use crate::bug_report::environment::{Host, build_environment_report};
use crate::bug_report::issue_url::{ISSUE_URL_BASE, MAX_URL_LENGTH, bug_report_issue_url};
use crate::bug_report::notice::{NOTICE_PREFIX, bug_report_notice, strip_bug_report_notice};
use crate::bug_report::providers::KNOWN_PROVIDERS;
use crate::bug_report::record::{BugReport, Deployment, EnvironmentReport, Surface};
use crate::bug_report::report::{CallContext, build_bug_report};

fn missing_host() -> Host<'static> {
    Host {
        dockerenv: Path::new("/nonexistent/.dockerenv"),
        python_version: None,
    }
}

fn report(frames: Vec<String>, config_lines: Vec<String>) -> BugReport {
    BugReport {
        environment: EnvironmentReport {
            surface: Surface::Sdk,
            litellm_version: String::from("1.2.3"),
            python_version: String::from("unknown"),
            deployment: Some(Deployment::PythonSdk),
            config_lines,
        },
        exception_type: String::from("Oops"),
        litellm_frames: frames,
        call_type: Some(String::from("acompletion")),
        custom_llm_provider: None,
        stream: Some(false),
    }
}

fn nested_backtrace(depth: u32) -> Backtrace {
    if depth == 0 {
        Backtrace::new()
    } else {
        nested_backtrace(depth.saturating_sub(1))
    }
}

#[rstest]
#[case::unset(None, true)]
#[case::lowercase_true(Some("true"), false)]
#[case::uppercase_true(Some("TRUE"), false)]
#[case::false_value(Some("false"), true)]
#[case::one(Some("1"), true)]
#[case::empty(Some(""), true)]
fn bug_report_enabled_reads_the_disable_flag(#[case] flag: Option<&str>, #[case] enabled: bool) {
    assert_eq!(bug_report_enabled(flag), enabled);
}

#[rstest]
#[case::plain_error(None, None, true)]
#[case::http_error(None, Some(500), false)]
#[case::disabled(Some("True"), None, false)]
fn bug_report_eligibility_needs_no_status(
    #[case] flag: Option<&str>,
    #[case] status: Option<u16>,
    #[case] eligible: bool,
) {
    assert_eq!(should_report_bug(flag, status), eligible);
}

#[rstest]
#[case::sdk(Surface::Sdk, "Cargo.toml", Some(Deployment::PythonSdk))]
#[case::proxy_in_docker(Surface::Proxy, "Cargo.toml", Some(Deployment::Docker))]
#[case::proxy_on_host(Surface::Proxy, "missing", None)]
fn bug_report_environment_detects_deployment(
    #[case] surface: Surface,
    #[case] marker: &str,
    #[case] deployment: Option<Deployment>,
) {
    let dockerenv = Path::new(env!("CARGO_MANIFEST_DIR")).join(marker);
    let config = vec![String::from("drop_params: true")];
    let host = Host {
        dockerenv: &dockerenv,
        python_version: Some("3.12.4"),
    };
    let environment = build_environment_report(surface, host, config.clone());
    assert_eq!(
        environment,
        EnvironmentReport {
            surface,
            litellm_version: env!("CARGO_PKG_VERSION").to_owned(),
            python_version: String::from("3.12.4"),
            deployment,
            config_lines: config
        }
    );
    assert_eq!(
        build_environment_report(surface, missing_host(), Vec::new()).python_version,
        "unknown"
    );
}

#[test]
fn bug_report_build_keeps_type_frames_and_allowlisted_provider() {
    let error = "x".parse::<u32>().unwrap_err();
    let context = CallContext {
        surface: Surface::Proxy,
        call_type: Some(String::from("aembedding")),
        custom_llm_provider: Some(String::from("openai")),
        stream: Some(true),
        config_lines: Vec::new(),
    };
    let built = build_bug_report(
        &error,
        &nested_backtrace(20),
        context.clone(),
        missing_host(),
    );
    assert_eq!(built.exception_type, "ParseIntError");
    assert_eq!(built.litellm_frames.len(), 12);
    let innermost = built.litellm_frames.last().unwrap();
    assert!(
        innermost.starts_with("crates/betterlite-reporting/src/bug_report/tests.rs:"),
        "{innermost}"
    );
    assert!(
        innermost.ends_with(" in betterlite_reporting::bug_report::tests::nested_backtrace"),
        "{innermost}"
    );
    assert_eq!(
        (built.call_type, built.stream, built.environment.surface),
        (Some(String::from("aembedding")), Some(true), Surface::Proxy)
    );
    assert_eq!(built.custom_llm_provider.as_deref(), Some("openai"));
    let unknown = CallContext {
        custom_llm_provider: Some(String::from("not-a-provider")),
        ..context
    };
    assert_eq!(
        build_bug_report(&error, &Backtrace::new(), unknown, missing_host()).custom_llm_provider,
        None
    );
    assert_eq!(KNOWN_PROVIDERS.split_ascii_whitespace().count(), 165);
}

#[test]
fn bug_report_issue_url_encodes_like_urlencode() {
    let config = vec![String::from("a=b&c d ~*")];
    let url = bug_report_issue_url(&report(
        vec![String::from("crates/x/src/y.rs:7 in x::y")],
        config,
    ));
    let expected_head = format!(
        "{ISSUE_URL_BASE}?template=bug_report.yml&labels=bug&title=%5BBug%5D%3A+Oops+in+crates%2Fx%2Fsrc%2Fy.rs&version=1.2.3&domain=Python+SDK%3A+the+litellm+package+itself&description=Auto-generated+by+LiteLLM"
    );
    assert!(url.starts_with(&expected_head), "{url}");
    assert!(url.contains("%0Aa%3Db%26c+d+~%2A%0A"), "{url}");
    assert!(url.contains("Endpoint+%2F+call%3A+acompletion%0AProvider%3A+unknown%0ALiteLLM%3A+1.2.3%0APython%3A+unknown%0AStream%3A+false%0A"), "{url}");
    assert!(url.ends_with("&deployment=pip+%2F+Python+SDK"), "{url}");
}

#[test]
fn bug_report_issue_url_drops_config_before_frames() {
    let frames: Vec<String> = (0_u8..12)
        .map(|index| format!("crates/x/src/f{index}.rs:1 in x::f"))
        .collect();
    let config: Vec<String> = (0_u16..300)
        .map(|index| format!("setting_{index:03}: true"))
        .collect();
    let url = bug_report_issue_url(&report(frames.clone(), config));
    assert!(url.len() <= MAX_URL_LENGTH);
    assert!(
        url.contains("f0.rs") && url.contains("setting_000") && !url.contains("setting_299"),
        "{url}"
    );
    let long_frames: Vec<String> = (0_u8..12)
        .map(|index| format!("crates/x/src/{index:02}{}.rs:1 in x", "y".repeat(600)))
        .collect();
    let trimmed = bug_report_issue_url(&report(long_frames, vec![String::from("setting: true")]));
    assert!(
        trimmed.len() <= MAX_URL_LENGTH
            && !trimmed.contains("setting")
            && !trimmed.contains("%2F00y")
            && trimmed.contains("%2F11y"),
        "{trimmed}"
    );
    let oversized = BugReport {
        exception_type: "E".repeat(7_000),
        ..report(frames, Vec::new())
    };
    let fallback = bug_report_issue_url(&oversized);
    assert!(fallback.len() > MAX_URL_LENGTH && !fallback.contains("f0.rs"));
}

#[rstest]
#[case::with_notice(
    "boom\nThis looks like a bug in LiteLLM rather than in your request. File it",
    "boom"
)]
#[case::one_newline_only(
    "boom\n\nThis looks like a bug in LiteLLM rather than in your request.",
    "boom\n"
)]
#[case::notice_only("This looks like a bug in LiteLLM rather than in your request.", "")]
#[case::no_notice("boom\n", "boom\n")]
fn bug_report_notice_strips_from_the_prefix_on(#[case] message: &str, #[case] stripped: &str) {
    assert_eq!(strip_bug_report_notice(message), stripped);
}

#[test]
fn bug_report_notice_ends_with_the_issue_url() {
    let report = report(Vec::new(), Vec::new());
    let notice = bug_report_notice(&report);
    assert!(
        notice.starts_with(NOTICE_PREFIX)
            && notice.ends_with(&format!(": {}", bug_report_issue_url(&report)))
    );
    let json = serde_json::to_string(&report.environment).unwrap();
    assert!(
        json.contains(r#""surface":"sdk""#) && json.contains(r#""deployment":"pip / Python SDK""#),
        "{json}"
    );
    assert_eq!(
        serde_json::from_str::<EnvironmentReport>(&json).unwrap(),
        report.environment
    );
}
