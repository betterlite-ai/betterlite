use rstest::rstest;

use crate::tag_matchers::candidates::{MatchVia, is_valid_deployment_tag};
use crate::tag_matchers::patterns::WarningKind;
use crate::tag_matchers::selection::TagMatcher;
use crate::tag_matchers::tokens::{bare_tag_value, split_tags};

use super::support::{deployment, query, set, strings};

fn tagged() -> TagMatcher {
    TagMatcher::new(vec![
        deployment(Some(&["alpha", "beta"]), Some(&["^User-Agent: internal"])),
        deployment(Some(&["beta", "gamma"]), None),
        deployment(None, Some(&["^User-Agent: claude-code/"])),
        deployment(Some(&["default"]), None),
    ])
}

#[derive(Clone, Copy)]
enum Behavior {
    AnyAndAll,
    TagValidity,
    StrictBlock,
    HeaderPatterns,
    RequiredExcluded,
    RoutingPrefix,
    DefaultTags,
    KnownUnknown,
    Inherited,
    Markers,
    WarningLabels,
}

#[rstest]
#[case::any_and_all(Behavior::AnyAndAll)]
#[case::tag_validity(Behavior::TagValidity)]
#[case::strict_block(Behavior::StrictBlock)]
#[case::header_patterns(Behavior::HeaderPatterns)]
#[case::required_excluded(Behavior::RequiredExcluded)]
#[case::routing_prefix(Behavior::RoutingPrefix)]
#[case::default_tags(Behavior::DefaultTags)]
#[case::known_unknown(Behavior::KnownUnknown)]
#[case::inherited(Behavior::Inherited)]
#[case::markers(Behavior::Markers)]
#[case::warning_labels(Behavior::WarningLabels)]
fn tag_matchers_behaviors(#[case] behavior: Behavior) {
    match behavior {
        Behavior::AnyAndAll => any_and_all_matching(),
        Behavior::TagValidity => tag_validity_table(),
        Behavior::StrictBlock => strict_tags_block_regex(),
        Behavior::HeaderPatterns => header_patterns(),
        Behavior::RequiredExcluded => required_and_excluded(),
        Behavior::RoutingPrefix => routing_prefix(),
        Behavior::DefaultTags => default_tags(),
        Behavior::KnownUnknown => known_and_unknown_tags(),
        Behavior::Inherited => inherited_constraints_stay_separate(),
        Behavior::Markers => token_markers(),
        Behavior::WarningLabels => warning_labels_are_stable(),
    }
}

fn any_and_all_matching() {
    let matcher = tagged();
    let any = matcher.select(&query(&["beta"], None, &[], true, ""));
    assert_eq!(any.tags.positive, strings(&["beta"]));
    assert!(any.tags.required.is_empty());
    assert_eq!(any.candidates, vec![0, 1, 2, 3]);
    assert_eq!(any.matches.len(), 2);
    assert_eq!(any.matches[0].index, 0);
    assert_eq!(any.matches[0].via, MatchVia::Tags);
    assert_eq!(any.matches[0].value, "beta");
    assert_eq!(any.matches[1].index, 1);
    let all = matcher.select(&query(&["alpha", "beta"], None, &[], false, ""));
    assert_eq!(all.matches.len(), 1);
    assert_eq!(all.matches[0].value, "alpha");
}

fn tag_validity_table() {
    let deployment = strings(&["a", "b"]);
    assert!(is_valid_deployment_tag(&deployment, &strings(&["b"]), true));
    assert!(!is_valid_deployment_tag(
        &deployment,
        &strings(&["c"]),
        true
    ));
    assert!(is_valid_deployment_tag(
        &deployment,
        &strings(&["a", "b"]),
        false
    ));
    assert!(!is_valid_deployment_tag(
        &deployment,
        &strings(&["a", "c"]),
        false
    ));
    assert!(!is_valid_deployment_tag(&deployment, &[], true));
}

fn strict_tags_block_regex() {
    let matcher = tagged();
    let strict = matcher.select(&query(
        &["other"],
        None,
        &["User-Agent: internal"],
        false,
        "",
    ));
    assert!(strict.matches.is_empty());
    let lenient = matcher.select(&query(
        &["other"],
        None,
        &["User-Agent: internal"],
        true,
        "",
    ));
    assert_eq!(lenient.matches.len(), 1);
    assert_eq!(lenient.matches[0].via, MatchVia::TagRegex);
    assert_eq!(lenient.matches[0].value, "^User-Agent: internal");
    let header_only = matcher.select(&query(
        &[],
        None,
        &["User-Agent: claude-code/1.0"],
        false,
        "",
    ));
    assert_eq!(header_only.matches.len(), 1);
    assert_eq!(header_only.matches[0].index, 2);
    assert_eq!(header_only.matches[0].via, MatchVia::TagRegex);
}

fn header_patterns() {
    let matcher = TagMatcher::new(vec![deployment(
        None,
        Some(&["^User-Agent: other", "^User-Agent: claude-code/"]),
    )]);
    let selection = matcher.select(&query(
        &[],
        None,
        &["User-Agent: x", "User-Agent: claude-code/1.0"],
        true,
        "",
    ));
    assert_eq!(selection.matches.len(), 1);
    assert_eq!(selection.matches[0].value, "^User-Agent: claude-code/");
    let invalid = TagMatcher::new(vec![deployment(None, Some(&["(", "^User-Agent:"]))]);
    assert_eq!(invalid.warnings[0].kind, WarningKind::InvalidPattern);
    assert_eq!(invalid.warnings[0].pattern, "(");
    let invalid_selection = invalid.select(&query(&[], None, &["User-Agent: x"], true, ""));
    assert!(invalid_selection.warnings.is_empty());
    assert_eq!(invalid_selection.matches.len(), 1);
    assert_eq!(invalid_selection.matches[0].value, "^User-Agent:");
}

fn required_and_excluded() {
    let matcher = TagMatcher::new(vec![
        deployment(Some(&["team-a", "alpha"]), None),
        deployment(Some(&["team-a", "team-b"]), None),
        deployment(Some(&["alpha"]), None),
    ]);
    let selection = matcher.select(&query(
        &["&team-a", "!team-b", "alpha"],
        None,
        &[],
        true,
        "",
    ));
    assert_eq!(selection.tags.required, strings(&["team-a"]));
    assert_eq!(selection.tags.positive, strings(&["alpha"]));
    assert_eq!(selection.tags.excluded, strings(&["team-b"]));
    assert_eq!(selection.allowed, vec![0, 2]);
    assert_eq!(selection.candidates, vec![0]);
    assert_eq!(selection.matches.len(), 1);
    assert_eq!(selection.matches[0].value, "alpha");
}

fn routing_prefix() {
    let matcher = TagMatcher::new(vec![deployment(Some(&["alpha", "gamma"]), None)]);
    let selection = matcher.select(&query(
        &["group:alpha", "!group:beta", "group:&gamma", "plain"],
        None,
        &[],
        true,
        "group:",
    ));
    assert_eq!(
        selection.rewritten,
        strings(&["alpha", "!group:beta", "&gamma", "plain"])
    );
    assert_eq!(selection.tags.required, strings(&["gamma"]));
    assert_eq!(selection.tags.positive, strings(&["alpha", "plain"]));
    assert_eq!(selection.tags.excluded, strings(&["group:beta"]));
    assert_eq!(selection.routing_confirmed, set(&["alpha", "gamma"]));
    assert_eq!(selection.matches.len(), 1);
    assert_eq!(selection.matches[0].value, "alpha");
}

fn default_tags() {
    let selection = tagged().select(&query(&[], None, &[], true, ""));
    assert_eq!(selection.defaults, vec![3]);
    let without = TagMatcher::new(vec![
        deployment(Some(&["alpha"]), None),
        deployment(None, None),
    ]);
    assert_eq!(
        without.select(&query(&[], None, &[], true, "")).defaults,
        vec![0, 1]
    );
}

fn known_and_unknown_tags() {
    let matcher = tagged();
    assert_eq!(
        matcher.select(&query(&[], None, &[], true, "")).known_tags,
        set(&["alpha", "beta", "default", "gamma"])
    );
    let hidden = TagMatcher::new(vec![
        deployment(Some(&["known"]), None),
        deployment(Some(&["known", "other"]), None),
    ]);
    assert!(hidden.unknown_required_hides_answer(
        &[0, 1],
        &set(&[]),
        &set(&["known", "unknown"]),
        &set(&[])
    ));
    assert!(!hidden.unknown_required_hides_answer(
        &[0, 1],
        &set(&[]),
        &set(&["unknown"]),
        &set(&[])
    ));
    assert!(!hidden.unknown_required_hides_answer(
        &[0, 1],
        &set(&[]),
        &set(&["known", "other"]),
        &set(&[])
    ));
    let routed = TagMatcher::new(vec![deployment(Some(&["a"]), None)]);
    assert!(routed.unknown_required_hides_answer(&[0], &set(&[]), &set(&["a", "b"]), &set(&[])));
    assert!(!routed.unknown_required_hides_answer(
        &[0],
        &set(&[]),
        &set(&["a", "b"]),
        &set(&["b"])
    ));
    let selection = hidden.select(&query(&["&known", "&unknown"], None, &[], true, ""));
    assert!(selection.unknown_required_hides_answer);
}

fn inherited_constraints_stay_separate() {
    let matcher = tagged();
    let selection = matcher.select(&query(
        &["alpha"],
        Some(&["&plan", "!trial"]),
        &[],
        true,
        "",
    ));
    assert_eq!(selection.inherited_required, Some(set(&["plan"])));
    assert_eq!(selection.inherited_excluded, Some(set(&["trial"])));
    assert!(selection.tags.required.is_empty());
    assert_eq!(selection.tags.positive, strings(&["alpha"]));
    let absent = matcher.select(&query(&["alpha"], None, &[], true, ""));
    assert_eq!(absent.inherited_required, None);
    assert_eq!(absent.inherited_excluded, None);
}

fn warning_labels_are_stable() {
    assert_eq!(WarningKind::InvalidPattern.label(), "invalid_pattern");
    assert_eq!(
        WarningKind::UnsupportedPattern.label(),
        "unsupported_pattern"
    );
    assert_eq!(WarningKind::PatternTooLarge.label(), "pattern_too_large");
    assert_eq!(
        WarningKind::MatchLimitExceeded.label(),
        "match_limit_exceeded"
    );
}

fn token_markers() {
    assert_eq!(bare_tag_value("&team"), Some("team"));
    assert_eq!(bare_tag_value("!team"), Some("team"));
    assert_eq!(bare_tag_value("&"), None);
    assert_eq!(bare_tag_value("!"), None);
    assert_eq!(bare_tag_value("team"), Some("team"));
    let parts = split_tags(&strings(&["&a", "!b", "c", "&", "!", ""]));
    assert_eq!(parts.required, strings(&["a"]));
    assert_eq!(parts.excluded, strings(&["b"]));
    assert_eq!(parts.positive, strings(&["c", ""]));
}
