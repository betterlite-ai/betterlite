use crate::azure_content_safety::constants::{MAX_TEXT_LENGTH, TEXT_RECORD_LENGTH};
use crate::azure_content_safety::text_splitting::split_text_by_words;

#[test]
fn constants_match_the_azure_limits() {
    assert_eq!(MAX_TEXT_LENGTH, 10_000);
    assert_eq!(TEXT_RECORD_LENGTH, 1_000);
}

#[test]
fn returns_short_text_as_one_chunk() {
    assert_eq!(split_text_by_words("hello world", 11), ["hello world"]);
}

#[test]
fn returns_text_of_exactly_the_limit_as_one_chunk() {
    assert_eq!(split_text_by_words("abcdefghij", 10), ["abcdefghij"]);
}

#[test]
fn returns_an_empty_text_as_one_empty_chunk() {
    assert_eq!(split_text_by_words("", 10), [""]);
}

#[test]
fn splits_at_a_word_boundary_without_breaking_words() {
    assert_eq!(
        split_text_by_words("hello world", 5),
        ["hello", " ", "world"]
    );
}

#[test]
fn keeps_mixed_whitespace_in_its_own_chunk() {
    assert_eq!(
        split_text_by_words("hello \n\t world", 6),
        ["hello", " \n\t ", "world"]
    );
}

#[test]
fn force_splits_a_token_longer_than_the_limit() {
    assert_eq!(split_text_by_words("aaaaaaa", 5), ["aaaaa", "aa"]);
}

#[test]
fn never_breaks_a_word_across_chunks() {
    assert_eq!(split_text_by_words("ab cdefg", 4), ["ab ", "cdef", "g"]);
}

#[test]
fn splits_at_the_ten_thousand_character_limit() {
    let word = "a".repeat(MAX_TEXT_LENGTH);
    let text = format!("{word} {word}");
    assert_eq!(
        split_text_by_words(&text, MAX_TEXT_LENGTH),
        vec![word.clone(), " ".to_owned(), word]
    );
}

#[test]
fn counts_characters_not_bytes() {
    // Each character is two UTF-8 bytes, so a byte limit would cut one in half.
    let text = String::from_utf8(vec![0xC3, 0xA9, 0xC3, 0xA9, 0xC3, 0xA9]).unwrap();
    let two = String::from_utf8(vec![0xC3, 0xA9, 0xC3, 0xA9]).unwrap();
    let one = String::from_utf8(vec![0xC3, 0xA9]).unwrap();
    assert_eq!(split_text_by_words(&text, 2), vec![two, one]);
}

#[test]
fn counts_python_control_separators_as_whitespace() {
    // Python's \s also matches U+001C, which Rust's char::is_whitespace does not.
    assert_eq!(split_text_by_words("ab\u{1c}cd", 2), ["ab", "\u{1c}", "cd"]);
}

#[test]
fn a_zero_limit_splits_to_single_characters_instead_of_looping() {
    assert_eq!(split_text_by_words("abc", 0), ["a", "b", "c"]);
}

#[test]
fn concatenating_the_chunks_restores_the_text() {
    let text = "alpha beta\ngamma delta";
    assert_eq!(split_text_by_words(text, 11).concat(), text);
}
