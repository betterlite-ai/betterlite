#![cfg(test)]

use std::fs;
use std::path::Path;
use std::str;

use rstest::rstest;

use crate::error::PathError;
use crate::paths::delete::remove;
use crate::paths::lookup::find;
use crate::paths::segments::parse;
use crate::paths::{Document, PathLimits, Segment};

const LIMITS: PathLimits = PathLimits {
    max_path_bytes: 64,
    max_depth: 4,
    max_selectors: 2,
    max_visits: 16,
};

fn field(name: &str) -> Segment {
    Segment::Field(name.to_owned())
}

#[rstest]
#[case::fields("a.b", Ok(vec![field("a"), field("b")]))]
#[case::wildcard_then_field("a[*].b", Ok(vec![field("a"), Segment::Wildcard, field("b")]))]
#[case::index_without_dot("a[2]b", Ok(vec![field("a"), Segment::Index(2), field("b")]))]
#[case::leading_zeros("a[007]", Ok(vec![field("a"), Segment::Index(7)]))]
#[case::spaced_wildcard("a[ * ]", Ok(vec![field("a"), Segment::Unmatched]))]
#[case::signed_underscores("a[+1_0]", Ok(vec![field("a"), Segment::Index(10)]))]
#[case::trailing_underscore("a[1_]", Ok(vec![field("a"), Segment::Unmatched]))]
#[case::sign_then_space("a[- 1]", Ok(vec![field("a"), Segment::Unmatched]))]
#[case::unicode_space("a[\u{3000}3\u{85}]", Ok(vec![field("a"), Segment::Index(3)]))]
#[case::nested_open_bracket("[[0]]", Ok(vec![Segment::Unmatched, field("]")]))]
#[case::lone_open_bracket("[", Ok(vec![]))]
#[case::newline_in_field("a\nb", Ok(vec![field("a\nb")]))]
#[case::selectors_at_limit("[0][*]", Ok(vec![Segment::Index(0), Segment::Wildcard]))]
#[case::too_many_selectors("[0][*][x]", Err(PathError::TooManySelectors { limit: 2 }))]
#[case::too_deep("a.b.c.d.e", Err(PathError::TooDeep { limit: 4 }))]
#[case::long_digit_run(&format!("[{}]", "0".repeat(4301)), Ok(vec![Segment::Unmatched]))]
fn dot_paths_behavior_tests_behavior(
    #[case] path: &str,
    #[case] expected: Result<Vec<Segment>, PathError>,
) {
    let limits = PathLimits {
        max_path_bytes: 5000,
        ..LIMITS
    };
    assert_eq!(parse(&limits, path), expected);
}

#[rstest]
fn replays_the_control_paths_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/control_paths");
    let document: Document = serde_json::from_str(r#"{"a": [{"b": 1}], "c": {"d": 2}}"#).unwrap();
    for entry in fs::read_dir(corpus).unwrap() {
        let path = entry.unwrap().path();
        let bytes = fs::read(&path).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        let Ok(text) = str::from_utf8(&bytes) else {
            assert!(name.ends_with("-malformed"), "{name} is not UTF-8");
            continue;
        };
        let parsed = parse(&LIMITS, text);
        let _found = find(&LIMITS, &document, text);
        let _deleted = remove(&LIMITS, &document, text);
        assert_eq!(parsed.is_err(), name.ends_with("-rejected"), "{name}");
    }
}
