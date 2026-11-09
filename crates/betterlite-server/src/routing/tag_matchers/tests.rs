#![cfg(test)]

use betterlite_config::sections::tag_matchers::{DeploymentTags as ConfigDeployment, Settings};
use betterlite_router::tag_matchers::candidates::MatchVia;
use betterlite_router::tag_matchers::patterns::WarningKind;
use betterlite_router::tag_matchers::selection::TagQuery;
use betterlite_router::tag_matchers::tokens::RequestTags;

use super::resolve;

fn deployment(tags: Option<Vec<&str>>, regex: Option<Vec<&str>>) -> ConfigDeployment {
    ConfigDeployment {
        tags: tags.map(|values| values.into_iter().map(str::to_owned).collect()),
        tag_regex: regex.map(|values| values.into_iter().map(str::to_owned).collect()),
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
