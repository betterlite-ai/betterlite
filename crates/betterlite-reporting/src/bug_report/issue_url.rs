use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use crate::bug_report::description::issue_description;
use crate::bug_report::record::{BugReport, Surface};

pub const ISSUE_URL_BASE: &str = "https://github.com/BerriAI/litellm/issues/new";
pub const MAX_URL_LENGTH: usize = 6_000;

// Python's quote_plus keeps these bytes and turns spaces into plus signs afterwards
const QUERY_VALUE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b' ');

#[must_use]
pub fn bug_report_issue_url(report: &BugReport) -> String {
    let frames = report.litellm_frames.as_slice();
    let config_lines = report.environment.config_lines.as_slice();
    let no_config: &[String] = &[];
    let config_candidates = (0..=config_lines.len())
        .rev()
        .filter_map(|count| config_lines.get(..count))
        .map(|kept_config| (frames, kept_config));
    let frame_candidates = (1..=frames.len())
        .filter_map(|start| frames.get(start..))
        .map(|kept_frames| (kept_frames, no_config));
    config_candidates
        .chain(frame_candidates)
        .map(|(kept_frames, kept_config)| issue_url(report, kept_frames, kept_config))
        .find(|url| url.len() <= MAX_URL_LENGTH)
        .unwrap_or_else(|| issue_url(report, &[], no_config))
}

fn issue_url(report: &BugReport, frames: &[String], config_lines: &[String]) -> String {
    let location = frames.last().map_or("litellm", |frame| {
        frame
            .split_once(':')
            .map_or(frame.as_str(), |(path, _)| path)
    });
    let title = format!("[Bug]: {} in {location}", report.exception_type);
    let domain = match (report.environment.surface, &report.custom_llm_provider) {
        (Surface::Sdk, _) => "Python SDK: the litellm package itself",
        (Surface::Proxy, Some(_)) => "LLM translation: a specific provider's request or response",
        (Surface::Proxy, None) => "Proxy core: startup, config, health checks, endpoints",
    };
    let description = issue_description(report, frames, config_lines);
    let fields = [
        ("template", "bug_report.yml"),
        ("labels", "bug"),
        ("title", title.as_str()),
        ("version", report.environment.litellm_version.as_str()),
        ("domain", domain),
        ("description", description.as_str()),
    ];
    let deployment = report
        .environment
        .deployment
        .map(|deployment| ("deployment", deployment.as_str()));
    let query = fields
        .into_iter()
        .chain(deployment)
        .map(|(key, value)| {
            let encoded = utf8_percent_encode(value, QUERY_VALUE).to_string();
            format!("{key}={}", encoded.replace(' ', "+"))
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{ISSUE_URL_BASE}?{query}")
}
