use std::any::type_name_of_val;
use std::backtrace::Backtrace;
use std::error::Error;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

mod issue_description;
mod providers;

use providers::KNOWN_PROVIDERS;

const ISSUE_URL_BASE: &str = "https://github.com/BerriAI/litellm/issues/new";
const MAX_URL_LENGTH: usize = 6_000;
const MAX_FRAMES: usize = 12;
const DISABLE_ENV_VAR: &str = "LITELLM_DISABLE_BUG_REPORT_LINK";
const NOTICE_PREFIX: &str = "This looks like a bug in LiteLLM rather than in your request.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Surface {
    Sdk,
    Proxy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentReport {
    pub surface: Surface,
    pub litellm_version: String,
    pub python_version: String,
    pub deployment: Option<String>,
    pub config_lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BugReport {
    pub environment: EnvironmentReport,
    pub exception_type: String,
    pub litellm_frames: Vec<String>,
    pub call_type: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub stream: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BugReportOptions {
    pub surface: Surface,
    pub call_type: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub stream: Option<bool>,
    pub config_lines: Vec<String>,
}

#[must_use]
pub fn bug_report_enabled() -> bool {
    let Ok(environment) = fs::read_to_string("/proc/self/environ") else {
        return true;
    };
    environment
        .split('\0')
        .filter_map(|entry| entry.split_once('='))
        .find(|(name, _)| *name == DISABLE_ENV_VAR)
        .is_none_or(|(_, value)| !value.eq_ignore_ascii_case("true"))
}

#[must_use]
pub fn should_report_bug(status_code: Option<u16>) -> bool {
    let enabled = bug_report_enabled();
    let eligible = status_code.is_none();
    enabled && eligible
}

#[must_use]
pub fn build_environment_report(surface: Surface, config_lines: &[String]) -> EnvironmentReport {
    let deployment = match surface {
        Surface::Sdk => Some(String::from("pip / Python SDK")),
        Surface::Proxy => Path::new("/.dockerenv")
            .exists()
            .then(|| String::from("Docker")),
    };
    EnvironmentReport {
        surface,
        litellm_version: env!("CARGO_PKG_VERSION").to_owned(),
        python_version: String::from("unknown"),
        deployment,
        config_lines: config_lines.to_vec(),
    }
}

pub fn build_bug_report(
    exception: &(dyn Error + 'static),
    options: &BugReportOptions,
) -> BugReport {
    let mut frames = format_backtrace(&Backtrace::force_capture());
    frames.truncate(MAX_FRAMES);
    BugReport {
        environment: build_environment_report(options.surface, &options.config_lines),
        exception_type: type_name_of_val(exception)
            .rsplit("::")
            .next()
            .unwrap_or("Error")
            .to_owned(),
        litellm_frames: frames,
        call_type: options.call_type.clone(),
        custom_llm_provider: options
            .custom_llm_provider
            .as_deref()
            .filter(|value| KNOWN_PROVIDERS.contains(value))
            .map(str::to_owned),
        stream: options.stream,
    }
}

#[must_use]
pub fn bug_report_issue_url(report: &BugReport) -> String {
    let frames = &report.litellm_frames;
    let config_lines = &report.environment.config_lines;
    for count in (0..=config_lines.len()).rev() {
        let Some(candidate_config) = config_lines.get(..count) else {
            continue;
        };
        let url = issue_url(report, frames, candidate_config);
        if url.len() <= MAX_URL_LENGTH {
            return url;
        }
    }
    for start in 1..=frames.len() {
        let Some(candidate_frames) = frames.get(start..) else {
            continue;
        };
        let url = issue_url(report, candidate_frames, &[]);
        if url.len() <= MAX_URL_LENGTH {
            return url;
        }
    }
    issue_url(report, &[], &[])
}

#[must_use]
pub fn strip_bug_report_notice(message: &str) -> String {
    let Some(index) = message.find(NOTICE_PREFIX) else {
        return message.to_owned();
    };
    let (head, _) = message.split_at(index);
    head.strip_suffix('\n').unwrap_or(head).to_owned()
}

#[must_use]
pub fn bug_report_notice(report: &BugReport) -> String {
    let url = bug_report_issue_url(report);
    format!(
        "{NOTICE_PREFIX} File it with one click (prefilled, no request data or error text, review before submitting): {url}"
    )
}

fn format_backtrace(backtrace: &Backtrace) -> Vec<String> {
    let text = backtrace.to_string();
    text.lines()
        .filter_map(|raw_line| {
            let line = raw_line.trim();
            (line.contains("crates/betterlite") || line.contains("/betterlite/"))
                .then(|| line.to_owned())
        })
        .collect()
}

fn issue_url(report: &BugReport, frames: &[String], config_lines: &[String]) -> String {
    let deployment = report
        .environment
        .deployment
        .as_deref()
        .map(|value| format!("&deployment={}", form_encode(value)))
        .unwrap_or_default();
    let title_location = frames
        .last()
        .and_then(|frame| frame.split_once(':'))
        .map(|(location, _)| location)
        .map_or("litellm", |location| location);
    let domain = match (
        report.environment.surface,
        report.custom_llm_provider.as_deref(),
    ) {
        (Surface::Sdk, _) => "Python SDK: the litellm package itself",
        (Surface::Proxy, Some(_)) => "LLM translation: a specific provider's request or response",
        (Surface::Proxy, None) => "Proxy core: startup, config, health checks, endpoints",
    };
    let description = issue_description::description(report, frames, config_lines);
    let fields = [
        ("template", "bug_report.yml"),
        ("labels", "bug"),
        (
            "title",
            &format!("[Bug]: {} in {title_location}", report.exception_type),
        ),
        ("version", &report.environment.litellm_version),
        ("domain", domain),
        ("description", &description),
    ];
    let query = fields
        .iter()
        .map(|(key, value)| format!("{key}={}", form_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{ISSUE_URL_BASE}?{query}{deployment}")
}

fn form_encode(value: &str) -> String {
    let bytes = value.bytes();
    bytes.fold(String::new(), |mut encoded, byte| {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else if byte == b' ' {
            encoded.push('+');
        } else {
            encoded.push('%');
            encoded.push(hex_digit(byte >> 4_u8));
            encoded.push(hex_digit(byte & 0x0f_u8));
        }
        encoded
    })
}

fn hex_digit(value: u8) -> char {
    let index = usize::from(value);
    b"0123456789ABCDEF"
        .get(index)
        .map_or('?', |digit| char::from(*digit))
}
