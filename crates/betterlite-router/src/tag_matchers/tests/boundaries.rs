use rstest::rstest;

use crate::tag_matchers::candidates::MatchVia;
use crate::tag_matchers::document::{MAX_TAG_MATCH_BYTES, parse_tag_match};
use crate::tag_matchers::error::TagMatcherError;
use crate::tag_matchers::patterns::{MAX_MATCH_STEPS, MAX_PATTERN_BYTES, WarningKind};
use crate::tag_matchers::selection::TagMatcher;
use crate::tag_matchers::tokens::strip_routing_prefix;

use super::support::{deployment, query, set, strings};

#[derive(Clone)]
enum Boundary {
    PatternBytes(usize),
    DocumentBytes(usize),
    InvalidDocument,
    MatchSteps,
    FancyMatch,
    Unsupported(&'static str),
    Invalid(&'static str),
    Supported(&'static str),
    ManyGroups(usize, &'static str),
    EmptyPrefix,
    DefaultMatchAny,
    PooledKnown,
    Selection,
}

#[rstest]
#[case::pattern_at_limit(Boundary::PatternBytes(MAX_PATTERN_BYTES))]
#[case::pattern_over_limit(Boundary::PatternBytes(MAX_PATTERN_BYTES.saturating_add(1)))]
#[case::document_at_limit(Boundary::DocumentBytes(MAX_TAG_MATCH_BYTES))]
#[case::document_over_limit(Boundary::DocumentBytes(MAX_TAG_MATCH_BYTES.saturating_add(1)))]
#[case::invalid_document(Boundary::InvalidDocument)]
#[case::match_steps(Boundary::MatchSteps)]
#[case::fancy_match(Boundary::FancyMatch)]
#[case::named_escape(Boundary::Unsupported(r"\N{LATIN SMALL LETTER A}"))]
#[case::not_newline_escape(Boundary::Supported(r"\N"))]
#[case::word_start(Boundary::Unsupported(r"\<tag"))]
#[case::word_end(Boundary::Unsupported(r"tag\>"))]
#[case::posix_class(Boundary::Unsupported("[[:digit:]]+"))]
#[case::set_intersection(Boundary::Unsupported("[a&&b]"))]
#[case::set_symmetric_difference(Boundary::Unsupported("[a~~b]"))]
#[case::set_difference(Boundary::Unsupported("[a--b]"))]
#[case::nested_class(Boundary::Unsupported("[a[bc]]"))]
#[case::nested_class_at_start(Boundary::Unsupported("[[ab]]"))]
#[case::negated_nested_class_at_start(Boundary::Unsupported("[^[ab]]"))]
#[case::leading_zero_octal(Boundary::ManyGroups(12, r"\012"))]
#[case::three_digit_octal(Boundary::ManyGroups(101, r"\101"))]
#[case::two_digit_backreference(Boundary::Supported(r"()()()()()()()()()()()()\12"))]
#[case::backreference_then_digits(Boundary::Supported(r"(a)\1-1"))]
#[case::absolute_end(Boundary::Unsupported(r"a\Z"))]
#[case::conditional(Boundary::Unsupported("(?P<a>x)(?(a)y|z)"))]
#[case::verbose_mode(Boundary::Unsupported("(?x)a # note [x\n\\<tag"))]
#[case::verbose_x_after_flag_off(Boundary::Unsupported("(?x-i:a)"))]
#[case::verbose_flag_combined(Boundary::Unsupported("(?ix:a)"))]
#[case::verbose_flag_disabled(Boundary::Supported("(?-x:a)"))]
#[case::verbose_ascii_flag(Boundary::Unsupported("(?ax:a)"))]
#[case::verbose_multiline_flag(Boundary::Unsupported("(?mx:a)"))]
#[case::verbose_dotall_flag(Boundary::Unsupported("(?sx:a)"))]
#[case::verbose_unicode_flag(Boundary::Unsupported("(?ux:a)"))]
#[case::unterminated_flags(Boundary::Invalid("(?x"))]
#[case::inline_comment(Boundary::Unsupported(r"(?#[)\<tag"))]
#[case::class_escape_before_word(Boundary::Unsupported(r"[\d]\<tag"))]
#[case::caret_before_word(Boundary::Unsupported(r"[^^]\<tag"))]
#[case::caret_class(Boundary::Supported("[^^][ab]"))]
#[case::range_start(Boundary::Unsupported("[]-x]"))]
#[case::negated_range_start(Boundary::Unsupported("[^]-x]"))]
#[case::bracket_hyphen(Boundary::Supported("[]-]"))]
#[case::negated_bracket_hyphen(Boundary::Supported("[^]-]"))]
#[case::literal_bracket_then_range(Boundary::Supported("[]]-x]"))]
#[case::bracket_first_escape(Boundary::Supported(r"[]\<tag]"))]
#[case::negated_bracket_first_escape(Boundary::Supported(r"[^]\<tag]"))]
#[case::engine_refused(Boundary::Unsupported("(?~|abc|def)"))]
#[case::escaped_backslash_escape(Boundary::Supported(r"\\N\{"))]
#[case::escaped_backslash_bracket(Boundary::Supported(r"\\<tag"))]
#[case::literal_brace(Boundary::Supported(r"\t{"))]
#[case::escaped_digit(Boundary::Supported(r"\t0"))]
#[case::tab_escape_then_digits(Boundary::Supported(r"\t12"))]
#[case::set_operator_outside_class(Boundary::Supported("[a]&&"))]
#[case::single_ampersand(Boundary::Supported("[a&b]"))]
#[case::ampersand_first(Boundary::Supported("[&a]"))]
#[case::single_tilde(Boundary::Supported("[a~b]"))]
#[case::tilde_first(Boundary::Supported("[~a]"))]
#[case::letter_range(Boundary::Supported("[a-b]"))]
#[case::named_group(Boundary::Supported("(?P<a>x)"))]
#[case::escape_only_classes(Boundary::Supported(r"[\d][\d]"))]
#[case::host_pattern(Boundary::Supported(r"^Host: [\w\-]+\.[\w\-]+$"))]
#[case::case_insensitive_flag(Boundary::Supported("(?i)a"))]
#[case::empty_prefix(Boundary::EmptyPrefix)]
#[case::default_match_any(Boundary::DefaultMatchAny)]
#[case::pooled_known(Boundary::PooledKnown)]
#[case::selection(Boundary::Selection)]
fn tag_matchers_boundaries(#[case] boundary: Boundary) {
    match boundary {
        Boundary::PatternBytes(bytes) => pattern_bytes_boundary(bytes),
        Boundary::DocumentBytes(bytes) => document_bytes_boundary(bytes),
        Boundary::InvalidDocument => invalid_document_boundary(),
        Boundary::MatchSteps => match_steps_boundary(),
        Boundary::FancyMatch => fancy_match_boundary(),
        Boundary::Unsupported(pattern) => {
            warning_boundary(pattern, WarningKind::UnsupportedPattern);
        }
        Boundary::Invalid(pattern) => warning_boundary(pattern, WarningKind::InvalidPattern),
        Boundary::Supported(pattern) => supported_construct_boundary(pattern),
        Boundary::ManyGroups(groups, suffix) => many_groups_boundary(groups, suffix),
        Boundary::EmptyPrefix => empty_prefix_boundary(),
        Boundary::DefaultMatchAny => default_match_any_boundary(),
        Boundary::PooledKnown => pooled_known_boundary(),
        Boundary::Selection => selection_boundary(),
    }
}

fn pattern_bytes_boundary(bytes: usize) {
    let pattern = "a".repeat(bytes);
    let matcher = TagMatcher::new(vec![deployment(None, Some(&[pattern.as_str()]))]);
    let selection = matcher.select(&query(&[], None, &[pattern.as_str()], true, ""));
    if bytes > MAX_PATTERN_BYTES {
        assert_eq!(matcher.warnings.len(), 1);
        assert_eq!(matcher.warnings[0].kind, WarningKind::PatternTooLarge);
        assert_eq!(matcher.warnings[0].pattern.len(), bytes);
        assert!(selection.matches.is_empty());
        return;
    }
    assert!(matcher.warnings.is_empty());
    assert_eq!(selection.matches.len(), 1);
    assert_eq!(selection.matches[0].via, MatchVia::TagRegex);
}

fn document_bytes_boundary(bytes: usize) {
    let mut document = String::from("{\"deployments\":[]}");
    document.push_str(&" ".repeat(bytes.saturating_sub(document.len())));
    let outcome = parse_tag_match(document.as_bytes());
    if bytes > MAX_TAG_MATCH_BYTES {
        match outcome {
            Err(TagMatcherError::TooLarge {
                bytes: reported,
                limit,
            }) => {
                assert_eq!(reported, bytes);
                assert_eq!(limit, MAX_TAG_MATCH_BYTES);
            }
            other => panic!("expected a too-large document, got {other:?}"),
        }
        return;
    }
    let request = outcome.unwrap();
    assert!(request.deployments.is_empty());
}

fn invalid_document_boundary() {
    match parse_tag_match(b"{not json") {
        Err(TagMatcherError::Invalid { source }) => assert!(source.is_syntax()),
        other => panic!("expected an invalid document, got {other:?}"),
    }
}

fn default_match_any_boundary() {
    let request = parse_tag_match(b"{\"deployments\":[]}").unwrap();
    assert!(request.match_any);
}

fn fancy_match_boundary() {
    assert_eq!(MAX_PATTERN_BYTES, 2048);
    assert_eq!(MAX_MATCH_STEPS, 100_000);
    assert_eq!(MAX_TAG_MATCH_BYTES, 0x0010_0000);
    let backref = TagMatcher::new(vec![deployment(None, Some(&[r"(a)\1"]))]);
    assert!(backref.warnings.is_empty());
    let selection = backref.select(&query(&[], None, &["aa"], true, ""));
    assert_eq!(selection.matches.len(), 1);
    assert_eq!(selection.matches[0].value, r"(a)\1");
    assert!(selection.warnings.is_empty());
}

fn pooled_known_boundary() {
    let matcher = TagMatcher::new(vec![
        deployment(Some(&["known"]), None),
        deployment(Some(&["outside"]), None),
    ]);
    // the pool knows only "known", so "outside" is still the unknown tag that hides an answer
    assert!(matcher.unknown_required_hides_answer(
        &[0],
        &set(&[]),
        &set(&["known", "outside"]),
        &set(&[])
    ));
    // the pool holding only the "outside" deployment knows that tag and not "known"
    assert!(matcher.unknown_required_hides_answer(
        &[1],
        &set(&[]),
        &set(&["known", "outside"]),
        &set(&[])
    ));
    // excluding "known" empties the pool, so no answer is left to hide
    assert!(!matcher.unknown_required_hides_answer(
        &[0],
        &set(&["known"]),
        &set(&["known", "outside"]),
        &set(&[])
    ));
}

fn warning_boundary(pattern: &str, kind: WarningKind) {
    let matcher = TagMatcher::new(vec![deployment(None, Some(&[pattern]))]);
    assert_eq!(matcher.warnings.len(), 1);
    assert_eq!(matcher.warnings[0].kind, kind);
    assert_eq!(matcher.warnings[0].pattern, pattern);
    let selection = matcher.select(&query(&[], None, &["anything"], true, ""));
    assert!(selection.matches.is_empty());
    assert!(selection.warnings.is_empty());
}

fn supported_construct_boundary(pattern: &str) {
    let matcher = TagMatcher::new(vec![deployment(None, Some(&[pattern]))]);
    assert!(matcher.warnings.is_empty());
}

fn many_groups_boundary(groups: usize, suffix: &str) {
    let mut pattern = "()".repeat(groups);
    pattern.push_str(suffix);
    warning_boundary(&pattern, WarningKind::UnsupportedPattern);
}

fn match_steps_boundary() {
    let header = "ab".repeat(12);
    let matcher = TagMatcher::new(vec![deployment(None, Some(&["(?i)(a|b|ab)*(?>c)"]))]);
    assert!(matcher.warnings.is_empty());
    let selection = matcher.select(&query(&[], None, &[header.as_str()], true, ""));
    assert_eq!(selection.warnings.len(), 1);
    assert_eq!(selection.warnings[0].kind, WarningKind::MatchLimitExceeded);
    assert_eq!(selection.warnings[0].pattern, "(?i)(a|b|ab)*(?>c)");
    assert!(selection.matches.is_empty());
}

fn empty_prefix_boundary() {
    let (rewritten, confirmed) = strip_routing_prefix(&strings(&["a", "&b"]), "");
    assert_eq!(rewritten, strings(&["a", "&b"]));
    assert!(confirmed.is_empty());
    let (stripped, stripped_confirmed) = strip_routing_prefix(&strings(&["group:"]), "group:");
    assert_eq!(stripped, strings(&[""]));
    assert_eq!(stripped_confirmed, set(&[""]));
}

fn selection_boundary() {
    let matcher = TagMatcher::new(vec![
        deployment(Some(&["a", "shared"]), None),
        deployment(None, None),
    ]);
    let exact = matcher.select(&query(&["a"], None, &[], true, ""));
    assert_eq!(exact.matches.len(), 1);
    assert_eq!(exact.matches[0].index, 0);
    assert_eq!(exact.matches[0].value, "a");
    let superset = matcher.select(&query(&["a", "b"], None, &[], false, ""));
    assert!(superset.matches.is_empty());
    let duplicate = matcher.select(&query(&["a", "a"], None, &[], false, ""));
    assert_eq!(duplicate.matches.len(), 1);
    let unconstrained = matcher.select(&query(&[], None, &[], true, ""));
    assert_eq!(unconstrained.allowed, vec![0, 1]);
    assert_eq!(unconstrained.candidates, vec![0, 1]);
    let marker = matcher.select(&query(&["group:&"], None, &[], true, "group:"));
    assert!(marker.tags.required.is_empty());
    assert!(marker.tags.positive.is_empty());
    assert!(marker.tags.excluded.is_empty());
    assert!(marker.routing_confirmed.is_empty());
}
