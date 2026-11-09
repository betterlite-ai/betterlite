#![cfg(test)]

use std::io;
use std::str;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::tag_matchers::{DeploymentTags as ConfigDeployment, Settings};
use betterlite_router::tag_matchers::candidates::MatchVia;
use betterlite_router::tag_matchers::patterns::WarningKind;
use betterlite_router::tag_matchers::selection::TagQuery;
use betterlite_router::tag_matchers::tokens::RequestTags;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt::MakeWriter;

use super::resolve;

fn deployment(tags: Option<Vec<&str>>, regex: Option<Vec<&str>>) -> ConfigDeployment {
    ConfigDeployment {
        tags: tags.map(|values| values.into_iter().map(str::to_owned).collect()),
        tag_regex: regex.map(|values| values.into_iter().map(str::to_owned).collect()),
    }
}

#[derive(Clone)]
struct CaptureWriter(Arc<Mutex<String>>);

impl io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = str::from_utf8(buf)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        self.0
            .lock()
            .map_err(|source| io::Error::other(source.to_string()))?
            .push_str(text);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CaptureWriter {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

fn query(tags: &[&str], headers: &[&str]) -> TagQuery {
    TagQuery {
        request: RequestTags {
            caller: Some(tags.iter().map(|tag| (*tag).to_owned()).collect()),
            inherited: None,
        },
        headers: headers.iter().map(|header| (*header).to_owned()).collect(),
        match_any: true,
        routing_prefix: String::new(),
    }
}

#[test]
fn configured_tags_select_the_matching_deployment() {
    let matcher = resolve(&Settings {
        deployments: vec![
            deployment(Some(vec!["team-a"]), None),
            deployment(Some(vec!["team-b"]), None),
        ],
    });
    let selection = matcher.select(&query(&["team-b"], &[]));
    assert_eq!(selection.allowed, vec![0, 1]);
    assert_eq!(selection.candidates, vec![0, 1]);
    assert_eq!(selection.matches.len(), 1);
    assert_eq!(selection.matches[0].index, 1);
    assert_eq!(selection.matches[0].via, MatchVia::Tags);
    assert_eq!(selection.matches[0].value, "team-b");
}

#[test]
fn configured_patterns_match_headers() {
    let matcher = resolve(&Settings {
        deployments: vec![deployment(None, Some(vec!["^User-Agent: claude-code/"]))],
    });
    let selection = matcher.select(&query(&[], &["User-Agent: claude-code/1.0"]));
    assert_eq!(selection.matches.len(), 1);
    assert_eq!(selection.matches[0].via, MatchVia::TagRegex);
    assert_eq!(selection.matches[0].value, "^User-Agent: claude-code/");
}

#[test]
fn invalid_patterns_warn_at_resolve_and_never_match() {
    let matcher = resolve(&Settings {
        deployments: vec![deployment(None, Some(vec!["("]))],
    });
    assert_eq!(matcher.warnings.len(), 1);
    assert_eq!(matcher.warnings[0].kind, WarningKind::InvalidPattern);
    assert_eq!(matcher.warnings[0].pattern, "(");
    let selection = matcher.select(&query(&[], &["User-Agent: anything"]));
    assert!(selection.matches.is_empty());
}

#[test]
fn an_oversized_pattern_warns_and_is_skipped() {
    let pattern = "a".repeat(2049);
    let header = "a".repeat(2049);
    let matcher = resolve(&Settings {
        deployments: vec![deployment(None, Some(vec![pattern.as_str()]))],
    });
    assert_eq!(matcher.warnings.len(), 1);
    assert_eq!(matcher.warnings[0].kind, WarningKind::PatternTooLarge);
    let selection = matcher.select(&query(&[], &[header.as_str()]));
    assert!(selection.matches.is_empty());
}

#[test]
fn resolve_logs_each_skipped_pattern_once() {
    let records = Arc::new(Mutex::new(String::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(CaptureWriter(Arc::clone(&records)))
        .finish();
    let matcher = with_default(subscriber, || {
        resolve(&Settings {
            deployments: vec![deployment(
                None,
                Some(vec!["(", r"\N{LATIN SMALL LETTER A}"]),
            )],
        })
    });
    assert_eq!(matcher.warnings.len(), 2);
    let captured = records.lock().unwrap();
    assert!(captured.contains("invalid_pattern"), "{captured}");
    assert!(captured.contains("unsupported_pattern"), "{captured}");
    assert!(captured.contains(r"\N{LATIN SMALL LETTER A}"), "{captured}");
    assert_eq!(captured.matches("tag matcher skipped").count(), 2);
}
