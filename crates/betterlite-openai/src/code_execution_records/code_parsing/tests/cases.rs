#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::path::PathBuf;

#[cfg(test)]
use rstest::rstest;

#[cfg(test)]
#[rstest]
#[case::valid_code("{\"code\": \"print(1)\"}", "print(1)")]
#[case::empty_string("", "")]
#[case::malformed_json("not json", "")]
#[case::missing_code_field("{}", "")]
#[case::non_object_top_level("[1, 2]", "")]
#[case::non_string_code_value("{\"code\": 5}", "")]
fn parse_code_extracts_the_code_field_without_panicking(
    #[case] arguments: &str,
    #[case] expected: &str,
) {
    assert_eq!(parse_code(arguments), expected);
}

#[cfg(test)]
#[rstest]
fn replays_the_code_execution_arguments_corpus(
    #[files("fuzz/corpus/code_execution_arguments/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let Ok(arguments) = String::from_utf8(bytes) else {
        return;
    };
    let _parsed = parse_code(&arguments);
}
