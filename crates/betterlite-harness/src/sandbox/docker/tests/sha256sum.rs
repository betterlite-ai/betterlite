use std::fs;
use std::path::Path;

use crate::sandbox::docker::snapshot::parse_sha256sum;

fn hash(character: char) -> String {
    character.to_string().repeat(64)
}

#[test]
fn parses_sha256sum_lines_and_strips_one_leading_dot_slash() {
    let first = hash('a');
    let second = hash('b');
    let output = format!("{first}  ./src/main.rs\n{second}  lib.rs\nshort\n");
    let parsed = parse_sha256sum(output.as_bytes());
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed.get("src/main.rs"), Some(&first));
    assert_eq!(parsed.get("lib.rs"), Some(&second));
}

#[test]
fn skips_lines_of_exactly_sixty_six_characters() {
    let digest = hash('a');
    let output = format!("{digest}  \n{digest}  x\n");
    let parsed = parse_sha256sum(output.as_bytes());
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed.get("x"), Some(&digest));
}

#[test]
fn counts_characters_rather_than_bytes() {
    let digest = hash('a');
    let short = format!("{digest} \u{e9}\n");
    assert!(parse_sha256sum(short.as_bytes()).is_empty());
    let long = format!("{digest}  \u{e9}\n");
    let parsed = parse_sha256sum(long.as_bytes());
    assert_eq!(parsed.get("\u{e9}"), Some(&digest));
}

#[rstest::rstest]
#[case::next_line('\n')]
#[case::carriage_return('\r')]
#[case::line_tab('\u{b}')]
#[case::form_feed('\u{c}')]
#[case::file_separator('\u{1c}')]
#[case::group_separator('\u{1d}')]
#[case::record_separator('\u{1e}')]
#[case::next_line_nel('\u{85}')]
#[case::line_separator('\u{2028}')]
#[case::paragraph_separator('\u{2029}')]
fn splits_on_python_line_boundaries(#[case] separator: char) {
    let first = hash('a');
    let second = hash('b');
    let output = format!("{first}  ./one{separator}{second}  ./two");
    let parsed = parse_sha256sum(output.as_bytes());
    assert_eq!(parsed.len(), 2);
    assert!(parsed.contains_key("one"));
    assert!(parsed.contains_key("two"));
}

#[test]
fn keeps_backslash_escaped_names_as_written() {
    let digest = hash('a');
    let output = format!("{digest}  ./weird\\nname\n");
    let parsed = parse_sha256sum(output.as_bytes());
    assert_eq!(parsed.get("weird\\nname"), Some(&digest));
}

#[test]
fn decodes_invalid_utf8_with_replacement() {
    let digest = hash('a');
    let mut output = format!("{digest}  ./").into_bytes();
    output.push(0xff);
    output.push(b'\n');
    let parsed = parse_sha256sum(&output);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed.get("\u{fffd}"), Some(&digest));
}

#[test]
fn later_duplicate_keys_win() {
    let first = hash('a');
    let second = hash('b');
    let output = format!("{first}  ./same\n{second}  ./same\n");
    let parsed = parse_sha256sum(output.as_bytes());
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed.get("same"), Some(&second));
}

#[test]
fn ignores_empty_output() {
    assert!(parse_sha256sum(b"").is_empty());
    assert!(parse_sha256sum(b"\n\n").is_empty());
}

fn replay(folder: &Path) -> usize {
    let entries = fs::read_dir(folder).unwrap();
    let mut count = 0_usize;
    for entry in entries {
        let path = entry.unwrap().path();
        let bytes = fs::read(path).unwrap();
        drop(parse_sha256sum(&bytes));
        count = count.saturating_add(1);
    }
    count
}

#[test]
fn replays_the_docker_sha256sum_corpus() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/docker_sha256sum");
    assert!(replay(&folder) > 0);
}
