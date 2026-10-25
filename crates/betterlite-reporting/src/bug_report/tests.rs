#![cfg(test)]

use std::path::Path;

use backtrace::Backtrace;
use rstest::rstest;

use crate::bug_report::eligibility::{bug_report_enabled, should_report_bug};
use crate::bug_report::environment::{Host, build_environment_report};
use crate::bug_report::issue_url::{MAX_URL_LENGTH, bug_report_issue_url};
use crate::bug_report::notice::{NOTICE_PREFIX, bug_report_notice, strip_bug_report_notice};
use crate::bug_report::providers::KNOWN_PROVIDERS;
use crate::bug_report::record::{BugReport, Deployment, EnvironmentReport, Surface};
use crate::bug_report::report::{CallContext, build_bug_report};

fn no_docker() -> Host<'static> {
    Host {
        dockerenv: Path::new("/missing"),
        python_version: None,
    }
}

fn report(frames: Vec<String>, config_lines: Vec<String>) -> BugReport {
    let environment = build_environment_report(Surface::Sdk, no_docker(), config_lines);
    BugReport {
        environment,
        exception_type: String::from("Oops"),
        litellm_frames: frames,
        call_type: None,
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
#[case::unset(None, None, true, true)]
#[case::lowercase_true(Some("true"), None, false, false)]
#[case::mixed_case_true(Some("TrUe"), None, false, false)]
#[case::other_value(Some("1"), None, true, true)]
#[case::http_error(None, Some(500), true, false)]
fn bug_report_eligibility_reads_flag_and_status(
    #[case] flag: Option<&str>,
    #[case] status: Option<u16>,
    #[case] enabled: bool,
    #[case] eligible: bool,
) {
    assert_eq!(
        (bug_report_enabled(flag), should_report_bug(flag, status)),
        (enabled, eligible)
    );
}

#[rstest]
#[case::sdk(Surface::Sdk, "Cargo.toml", None, Some(Deployment::PythonSdk))]
#[case::proxy_in_docker(Surface::Proxy, "Cargo.toml", Some("3.12.4"), Some(Deployment::Docker))]
#[case::proxy_on_host(Surface::Proxy, "missing", None, None)]
fn bug_report_environment_detects_deployment(
    #[case] surface: Surface,
    #[case] marker: &str,
    #[case] python: Option<&str>,
    #[case] deployment: Option<Deployment>,
) {
    let marker_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(marker);
    let host = Host {
        dockerenv: &marker_path,
        python_version: python,
    };
    let environment = build_environment_report(surface, host, vec![String::from("a: true")]);
    let python_version = python.unwrap_or("unknown").to_owned();
    let litellm_version = env!("CARGO_PKG_VERSION").to_owned();
    assert_eq!(
        environment,
        EnvironmentReport {
            surface,
            litellm_version,
            python_version,
            deployment,
            config_lines: vec![String::from("a: true")]
        }
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
    let built = build_bug_report(&error, &nested_backtrace(20), context.clone(), no_docker());
    let innermost = built.litellm_frames.last().unwrap();
    assert!(
        innermost.starts_with("crates/betterlite-reporting/src/bug_report/tests.rs:")
            && innermost.ends_with("nested_backtrace"),
        "{innermost}"
    );
    assert_eq!(
        (built.exception_type.as_str(), built.litellm_frames.len()),
        ("ParseIntError", 12)
    );
    assert_eq!(
        (built.call_type, built.custom_llm_provider, built.stream),
        (
            context.call_type.clone(),
            context.custom_llm_provider.clone(),
            Some(true)
        )
    );
    let unknown = CallContext {
        custom_llm_provider: Some(String::from("not-a-provider")),
        ..context
    };
    assert_eq!(
        build_bug_report(&error, &Backtrace::new(), unknown, no_docker()).custom_llm_provider,
        None
    );
    assert_eq!(KNOWN_PROVIDERS.split_ascii_whitespace().count(), 165);
}

#[test]
fn bug_report_issue_url_encodes_like_urlencode() {
    let url = bug_report_issue_url(&report(
        vec![String::from("crates/x/y.rs:7 in x")],
        vec![String::from("a=b&c d ~*")],
    ));
    let version = env!("CARGO_PKG_VERSION");
    let head = format!(
        "https://github.com/BerriAI/litellm/issues/new?template=bug_report.yml&labels=bug&title=%5BBug%5D%3A+Oops+in+crates%2Fx%2Fy.rs&version={version}&domain=Python+SDK%3A+the+litellm+package+itself&description=Auto-generated+by+LiteLLM"
    );
    assert!(url.starts_with(&head), "{url}");
    assert!(url.contains(&format!("call%3A+unknown%0AProvider%3A+unknown%0ALiteLLM%3A+{version}%0APython%3A+unknown%0AStream%3A+false%0A")), "{url}");
    assert!(
        url.contains("%0Aa%3Db%26c+d+~%2A%0A") && url.ends_with("&deployment=pip+%2F+Python+SDK"),
        "{url}"
    );
}

#[test]
fn bug_report_issue_url_drops_config_before_frames() {
    let frames: Vec<String> = (0_u8..12)
        .map(|index| format!("crates/x/f{index}.rs:1 in x"))
        .collect();
    let config: Vec<String> = (0_u16..300)
        .map(|index| format!("setting_{index:03}: true"))
        .collect();
    let url = bug_report_issue_url(&report(frames.clone(), config));
    assert!(
        url.len() <= MAX_URL_LENGTH
            && url.contains("f0.rs")
            && url.contains("setting_000")
            && !url.contains("setting_299")
    );
    let long_frames: Vec<String> = (0_u8..12)
        .map(|index| format!("crates/{index:02}{}.rs:1 in x", "y".repeat(600)))
        .collect();
    let trimmed = bug_report_issue_url(&report(long_frames, vec![String::from("setting: true")]));
    assert!(
        trimmed.len() <= MAX_URL_LENGTH
            && !trimmed.contains("setting")
            && !trimmed.contains("%2F00y")
            && trimmed.contains("%2F11y")
    );
    let fallback = bug_report_issue_url(&BugReport {
        exception_type: "E".repeat(7_000),
        ..report(frames, Vec::new())
    });
    assert!(fallback.len() > MAX_URL_LENGTH && !fallback.contains("f0.rs"));
}

#[rstest]
#[case::with_notice(
    "boom\nThis looks like a bug in LiteLLM rather than in your request. File",
    "boom"
)]
#[case::one_newline_only(
    "boom\n\nThis looks like a bug in LiteLLM rather than in your request.",
    "boom\n"
)]
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
