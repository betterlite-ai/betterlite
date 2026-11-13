use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, SecondsFormat, Utc};
use rust_decimal::{Decimal, RoundingStrategy};

use super::error::DebugError;
use super::wire::{ErrorInformation, PassthroughJson, RequestResponsePayload, SpendLogRow};

const MIN_FENCE_LENGTH: usize = 3;
const BODIES_NOTE: &str = "Bodies are included for failed turns and the most recent turns. Bodies are empty unless the proxy runs with `general_settings.store_prompts_in_spend_logs: true`.";

pub trait Clock {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

pub struct ReportContext<'report> {
    pub session_id: &'report str,
    pub base_url: &'report str,
    pub max_body_chars: usize,
    pub clock: &'report dyn Clock,
}

pub fn build_report(
    context: &ReportContext<'_>,
    rows: &[SpendLogRow],
    payloads: &BTreeMap<String, Option<RequestResponsePayload>>,
) -> Result<String, DebugError> {
    if rows.is_empty() {
        return Err(DebugError::NoSessionRows {
            session_id: context.session_id.to_owned(),
            base_url: context.base_url.to_owned(),
        });
    }
    render_report(context, rows, payloads)
}

pub fn render_report(
    context: &ReportContext<'_>,
    rows: &[SpendLogRow],
    payloads: &BTreeMap<String, Option<RequestResponsePayload>>,
) -> Result<String, DebugError> {
    let summary = summary_lines(context, rows)?;
    let sections: Vec<String> = rows
        .iter()
        .zip(1_usize..)
        .map(|(row, index)| {
            row_section(
                row,
                index,
                payloads.get(&row.request_id).and_then(Option::as_ref),
                context.max_body_chars,
            )
        })
        .collect();
    Ok(format!("{}{}\n", summary.join("\n"), sections.join("\n\n")))
}

fn summary_lines(
    context: &ReportContext<'_>,
    rows: &[SpendLogRow],
) -> Result<Vec<String>, DebugError> {
    let total = total_spend(rows)?;
    let models = model_list(rows);
    Ok(vec![
        format!(
            "# LiteLLM debug report: Claude Code session `{}`",
            context.session_id
        ),
        String::new(),
        format!("- proxy: {}", context.base_url),
        format!(
            "- generated: {}",
            context
                .clock
                .now()
                .to_rfc3339_opts(SecondsFormat::Secs, false)
        ),
        format!(
            "- turns: {}, failed: {}",
            rows.len(),
            rows.iter().filter(|row| row.failed()).count()
        ),
        format!(
            "- total spend: ${:.6}",
            total.round_dp_with_strategy(6, RoundingStrategy::MidpointAwayFromZero)
        ),
        format!("- models: {models}"),
        String::new(),
        String::from(BODIES_NOTE),
        String::new(),
        String::from("## Turns"),
        String::new(),
    ])
}

fn total_spend(rows: &[SpendLogRow]) -> Result<Decimal, DebugError> {
    let mut total = Decimal::ZERO;
    for row in rows {
        total = total
            .checked_add(row.spend)
            .ok_or(DebugError::SpendOverflow)?;
    }
    Ok(total)
}

fn model_list(rows: &[SpendLogRow]) -> String {
    let model_names: BTreeSet<&str> = rows.iter().map(SpendLogRow::model_label).collect();
    if model_names.is_empty() {
        return String::from("n/a");
    }
    model_names.into_iter().collect::<Vec<_>>().join(", ")
}

fn row_section(
    row: &SpendLogRow,
    index: usize,
    payload: Option<&RequestResponsePayload>,
    max_chars: usize,
) -> String {
    let failed = row.failed();
    let status = if failed { "FAILED" } else { "ok" };
    let mut lines = vec![
        format!("### {index}. {status} {}", row.model_label()),
        format!("- request_id: `{}`", row.request_id),
        format!(
            "- time: {} -> {}",
            row.start_time.as_deref().unwrap_or("n/a"),
            row.end_time.as_deref().unwrap_or("n/a")
        ),
        format!(
            "- provider: {} ({}), call_type: {}",
            row.custom_llm_provider.as_deref().unwrap_or("?"),
            row.api_base.as_deref().unwrap_or("n/a"),
            row.call_type.as_deref().unwrap_or("?")
        ),
        format!(
            "- spend: ${:.6}, tokens: {} in / {} out",
            row.spend
                .round_dp_with_strategy(6, RoundingStrategy::MidpointAwayFromZero),
            row.prompt_tokens,
            row.completion_tokens
        ),
    ];
    if failed && let Some(error) = &row.metadata.error_information {
        push_error(&mut lines, error);
    }
    if let Some(bodies) = payload {
        push_bodies(&mut lines, bodies, max_chars);
    }
    lines.join("\n")
}

fn push_error(lines: &mut Vec<String>, error: &ErrorInformation) {
    let headline = format!(
        "- error: `{}` {}",
        error.error_code.as_deref().unwrap_or("?"),
        error.error_class.as_deref().unwrap_or_default()
    );
    lines.push(headline.trim_end().to_owned());
    lines.push(String::new());
    push_fenced(
        lines,
        error.error_message.as_deref().unwrap_or_default(),
        "",
    );
}

fn push_bodies(lines: &mut Vec<String>, payload: &RequestResponsePayload, max_chars: usize) {
    lines.push(String::new());
    lines.push(String::from("<details><summary>request body</summary>"));
    lines.push(String::new());
    push_fenced(
        lines,
        &truncated_json(&payload.proxy_server_request, max_chars),
        "json",
    );
    lines.push(String::from("</details>"));
    lines.push(String::new());
    lines.push(String::from("<details><summary>response</summary>"));
    lines.push(String::new());
    push_fenced(lines, &truncated_json(&payload.response, max_chars), "json");
    lines.push(String::from("</details>"));
}

fn truncated_json(value: &PassthroughJson, max_chars: usize) -> String {
    let text = value.text();
    let total = text.chars().count();
    if total <= max_chars {
        return text;
    }
    let kept: String = text.chars().take(max_chars).collect();
    format!(
        "{kept}\n... (truncated, {} more chars)",
        total.saturating_sub(max_chars)
    )
}

fn push_fenced(lines: &mut Vec<String>, text: &str, info: &str) {
    let longest_run = text
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat(longest_run.saturating_add(1).max(MIN_FENCE_LENGTH));
    lines.push(format!("{fence}{info}"));
    lines.push(text.to_owned());
    lines.push(fence);
}

mod tests;
