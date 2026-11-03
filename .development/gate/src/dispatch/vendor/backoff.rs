mod tests;

use std::process::{Command, Stdio};
use std::time::Duration;

pub(super) enum Cause {
    LoggedOut,
    Offline,
    Limit,
}

pub(super) fn cause(line: &str) -> Cause {
    if line.contains("Not logged in") || line.contains("authentication failed") {
        Cause::LoggedOut
    } else if line.contains("Can't reach the API server") || line.contains("EAI_AGAIN") {
        Cause::Offline
    } else {
        Cause::Limit
    }
}

pub(super) fn waiting(line: &str, age: Duration, fallback: Duration, now: u64) -> bool {
    match cause(line) {
        Cause::LoggedOut => true,
        Cause::Offline => age < Duration::from_secs(60),
        Cause::Limit => {
            let observed = now.saturating_sub(age.as_secs());
            reset(line, observed).map_or(age < fallback, |deadline| {
                if deadline < observed {
                    age < fallback
                } else {
                    now < deadline
                }
            })
        }
    }
}

pub(super) fn message(line: &str) -> String {
    let decoded = serde_json::from_str::<serde_json::Value>(line).ok();
    decoded
        .as_ref()
        .and_then(|value| value.get("result"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(line)
        .to_owned()
}

fn reset(line: &str, observed: u64) -> Option<u64> {
    let message = message(line);
    let raw = message
        .split_once("try again at ")
        .or_else(|| message.split_once("resets "))?
        .1;
    let first = raw
        .split(['\n', '\u{b7}'])
        .next()?
        .trim()
        .trim_end_matches('.');
    let (time, zone) = first.split_once('(').map_or((first, None), |(time, zone)| {
        (time.trim(), Some(zone.trim_end_matches(')')))
    });
    anchored_reset(time, zone, observed)
}

fn anchored_reset(time: &str, zone: Option<&str>, observed: u64) -> Option<u64> {
    let cleaned = time
        .split_whitespace()
        .map(clean_word)
        .collect::<Vec<_>>()
        .join(" ");
    let anchor = date(&format!("@{observed}"), "%F", zone)?;
    let explicit_year = time.split_whitespace().any(|word| {
        word.trim_end_matches(',').len() == 4 && word.trim_end_matches(',').parse::<u16>().is_ok()
    });
    let time_only = !time.contains(' ');
    let anchored = if time_only {
        format!("{anchor} {cleaned}")
    } else if explicit_year {
        cleaned
    } else {
        format!("{cleaned} {}", anchor.get(..4)?)
    };
    let deadline: u64 = date(&anchored, "%s", zone)?.parse().ok()?;
    if !explicit_year && deadline < observed {
        let adjustment = if time_only {
            "1 day"
        } else if observed.saturating_sub(deadline) > 180 * 86400 {
            "1 year"
        } else {
            return Some(deadline);
        };
        return date(&format!("{anchored} + {adjustment}"), "%s", zone)?
            .parse()
            .ok();
    }
    Some(deadline)
}

fn date(input: &str, format: &str, zone: Option<&str>) -> Option<String> {
    let mut command = Command::new("date");
    command
        .args(["--date", input, &format!("+{format}")])
        .stderr(Stdio::null());
    if let Some(timezone) = zone {
        command.env("TZ", timezone);
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}

fn clean_word(word: &str) -> &str {
    let part = word.trim_end_matches(',');
    if part
        .chars()
        .next()
        .is_some_and(|letter| letter.is_ascii_digit())
    {
        part.trim_end_matches("st")
            .trim_end_matches("nd")
            .trim_end_matches("rd")
            .trim_end_matches("th")
    } else {
        part
    }
}
