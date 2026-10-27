#![cfg(test)]
use rstest::rstest;

use crate::sandbox_backend::include_glob::{IncludeGlob, InvalidGlobPattern};

#[rstest]
#[case::basename_at_any_depth("*.py", "src/app/main.py", true)]
#[case::basename_other_extension("*.py", "src/app/main.rs", false)]
#[case::anchored_globstar("src/**/*.py", "src/app/main.py", true)]
#[case::globstar_matches_top_level("**/*.py", "main.py", true)]
#[case::leading_slash_anchors_to_root("/*.py", "top.py", true)]
#[case::leading_slash_skips_nested("/*.py", "src/app/main.py", false)]
#[case::star_skips_hidden_file("*", ".env", false)]
#[case::dot_pattern_matches_hidden_file(".*", ".env", true)]
#[case::bare_pattern_reaches_into_hidden_dirs("*.yml", ".github/workflows/ci.yml", true)]
#[case::globstar_skips_hidden_dirs("**/*.yml", ".github/workflows/ci.yml", false)]
#[case::star_stays_in_one_segment("src/*.py", "src/app/main.py", false)]
#[case::brace_alternative("*.{py,rs}", "lib.rs", true)]
#[case::brace_misses_other("*.{py,rs}", "lib.go", false)]
#[case::nested_braces("{a,{b,c}}.txt", "c.txt", true)]
#[case::brace_without_comma_is_literal("{a}.txt", "{a}.txt", true)]
#[case::unclosed_brace_is_literal("{a,b.txt", "{a,b.txt", true)]
#[case::case_sensitive("*.PY", "main.py", false)]
fn matches_like_the_deepagents_include_glob(
    #[case] pattern: &str,
    #[case] relative: &str,
    #[case] expected: bool,
) {
    let include = IncludeGlob::compile(pattern).unwrap();
    assert_eq!(
        include.matches("/work", &format!("/work/{relative}")),
        expected
    );
}

#[rstest]
#[case::parent_segment("../*.py")]
#[case::inner_parent_segment("src/../*.py")]
#[case::backslash_parent_segment("src\\..\\*.py")]
fn rejects_traversal_in_the_pattern(#[case] pattern: &str) {
    let error = IncludeGlob::compile(pattern).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("Path traversal not allowed in glob pattern '{pattern}'")
    );
}

#[test]
fn rejects_more_than_a_thousand_brace_expansions() {
    let pattern = "{a,b}".repeat(10);
    let error = IncludeGlob::compile(&pattern).unwrap_err();
    assert!(matches!(
        error,
        InvalidGlobPattern::TooManyExpansions { .. }
    ));
}

#[test]
fn rejects_a_pattern_the_matcher_cannot_compile() {
    let error = IncludeGlob::compile("src/**a").unwrap_err();
    assert!(matches!(error, InvalidGlobPattern::Pattern { .. }));
    assert!(
        error
            .to_string()
            .starts_with("Invalid glob pattern 'src/**a': ")
    );
}
