use rstest::rstest;

use crate::release_info::bullets::{ReleaseCounts, count_release_bullets, release_lines};

const NONE: ReleaseCounts = ReleaseCounts {
    new_features: 0,
    bug_fixes: 0,
    other_updates: 0,
};
const FEATURE: ReleaseCounts = ReleaseCounts {
    new_features: 1,
    bug_fixes: 0,
    other_updates: 0,
};
const FIX: ReleaseCounts = ReleaseCounts {
    new_features: 0,
    bug_fixes: 1,
    other_updates: 0,
};
const OTHER: ReleaseCounts = ReleaseCounts {
    new_features: 0,
    bug_fixes: 0,
    other_updates: 1,
};

#[rstest]
#[case::feat("* feat: add a button", FEATURE)]
#[case::feat_capitalized("* Feat: add a button", FEATURE)]
#[case::feat_uppercase("* FEAT: add a button", FEATURE)]
#[case::mixed_case("* fiX: stop a crash", FIX)]
#[case::fix("* fix: stop a crash", FIX)]
#[case::scope("* feat(ui): add a button", FEATURE)]
#[case::breaking("* feat!: add a button", FEATURE)]
#[case::scope_and_breaking("* fix(core)!: stop a crash", FIX)]
#[case::empty_scope("* feat(): add a button", FEATURE)]
#[case::leading_whitespace_run("* \t feat: add a button", FEATURE)]
#[case::tab_after_the_asterisk("*\tfeat: add a button", FEATURE)]
#[case::unit_separator_after_the_colon("* fix:\u{1f}stop a crash", FIX)]
#[case::unit_separator_is_not_a_line_boundary("* fix: stop\u{1f}* feat: add", FIX)]
#[case::other_prefix("* chore: bump dependencies", OTHER)]
#[case::unprefixed_bullet("* add a button", OTHER)]
#[case::no_space_after_the_colon("* feat:add a button", OTHER)]
#[case::two_spaces_after_the_colon("* feat:  add a button", OTHER)]
#[case::nothing_after_the_colon("* feat:", OTHER)]
#[case::only_a_space_after_the_colon("* feat: ", OTHER)]
#[case::nested_open_paren_in_the_scope("* fix(a(b): stop a crash", FIX)]
#[case::text_after_the_scope("* fix(a(b)): stop a crash", OTHER)]
#[case::scope_before_the_bang("* feat!(ui): add a button", OTHER)]
#[case::text_in_front_of_the_prefix("* (ui) feat: add a button", OTHER)]
#[case::first_contribution("* @ann made their first contribution", NONE)]
#[case::first_contribution_with_punctuation("* @ann made their first contribution.", NONE)]
#[case::first_contribution_without_a_word_boundary("* @ann made their first contribution2", OTHER)]
#[case::plural_contribution_text("* @ann made their first contributions", OTHER)]
#[case::prefixed_first_contribution_text("* feat: @ann made their first contribution", FEATURE)]
#[case::no_space_after_the_asterisk("*feat: add a button", NONE)]
#[case::indented_bullet(" * feat: add a button", NONE)]
#[case::hyphen_list("- feat: add a button", NONE)]
#[case::double_asterisk("** feat: add a button", NONE)]
#[case::only_whitespace_after_the_asterisk("*   ", NONE)]
#[case::empty_body("", NONE)]
#[case::text_without_bullets("no bullets at all", NONE)]
#[case::two_buckets("* fix: stop\n* feat: add", ReleaseCounts {
    new_features: 1,
    bug_fixes: 1,
    other_updates: 0,
})]
#[case::carriage_return_line_feed("* fix: stop\r\n* feat: add\r\n", ReleaseCounts {
    new_features: 1,
    bug_fixes: 1,
    other_updates: 0,
})]
#[case::lone_carriage_return("* fix: stop\r* feat: add", ReleaseCounts {
    new_features: 1,
    bug_fixes: 1,
    other_updates: 0,
})]
#[case::line_separator("* fix: stop\u{2028}* feat: add", ReleaseCounts {
    new_features: 1,
    bug_fixes: 1,
    other_updates: 0,
})]
#[case::file_separator("* fix: stop\u{1c}* feat: add", ReleaseCounts {
    new_features: 1,
    bug_fixes: 1,
    other_updates: 0,
})]
#[case::next_line("* fix: stop\u{85}* feat: add", ReleaseCounts {
    new_features: 1,
    bug_fixes: 1,
    other_updates: 0,
})]
#[case::first_contribution_before_a_bullet(
    "* @ann made their first contribution\n* fix: stop",
    FIX
)]
#[case::no_author_name("* @ made their first contribution", OTHER)]
#[case::non_breaking_space_after_the_asterisk("*\u{a0}fix: stop a crash", FIX)]
#[case::unbulleted_first_contribution("@ann made their first contribution", NONE)]
fn counts_each_bullet_once(#[case] body: &str, #[case] expected: ReleaseCounts) {
    assert_eq!(count_release_bullets(body), expected);
}

// Python's str.splitlines keeps no trailing empty line and pairs a carriage return with the feed that follows it
#[test]
fn splits_lines_the_way_python_does() {
    assert_eq!(release_lines("a\nb\r\nc\rd"), ["a", "b", "c", "d"]);
    assert_eq!(release_lines("a\n"), ["a"]);
    assert_eq!(release_lines("a\r\n"), ["a"]);
    assert_eq!(release_lines("a\r"), ["a"]);
    assert_eq!(release_lines("\n"), [""]);
    assert!(release_lines("").is_empty());
}
