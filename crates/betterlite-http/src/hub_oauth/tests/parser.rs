use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::hub_oauth::stored_tokens::parse::{ParseError, parse};

type Parsed = Vec<(String, Vec<(String, String)>)>;

fn items(text: &str) -> Parsed {
    parse(text.as_bytes())
        .unwrap()
        .with_defaults()
        .into_iter()
        .map(|section| {
            let fields = section
                .fields
                .iter()
                .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
                .collect();
            (section.name, fields)
        })
        .collect()
}

fn one(name: &str, fields: &[(&str, &str)]) -> Parsed {
    vec![(
        name.to_owned(),
        fields
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
    )]
}

// every expectation below is what Python 3.13's ConfigParser(interpolation=None) returns from items()
#[rstest]
#[case::delimiters_and_key_case("[a]\nk = v\nK2: v2\n", "a", &[("k", "v"), ("k2", "v2")])]
#[case::comments("# top\n[a]\n; semi\nk = v # kept\n   # indented\nj=1\n", "a", &[("k", "v # kept"), ("j", "1")])]
#[case::continuation_with_empty_line("[a]\nk = line1\n  line2\n\n  line3\nj = x\n", "a", &[("k", "line1\nline2\n\nline3"), ("j", "x")])]
#[case::trailing_blank_lines("[a]\nk = v\n\n\nj = w\n", "a", &[("k", "v"), ("j", "w")])]
#[case::percent_is_kept("[a]\nk = 100%\nj = %(x)s\n", "a", &[("k", "100%"), ("j", "%(x)s")])]
#[case::header_text_after_bracket("[a] trailing\nk=v\n", "a", &[("k", "v")])]
#[case::header_up_to_last_bracket("[a]b]\nk=v\n", "a]b", &[("k", "v")])]
#[case::header_spaces_stay("[ a b ]\nk=v\n", " a b ", &[("k", "v")])]
#[case::crlf("[a]\r\nk = v\r\nj = w\r\n", "a", &[("k", "v"), ("j", "w")])]
#[case::lone_cr("[a]\rk = v\rj = w\r", "a", &[("k", "v"), ("j", "w")])]
#[case::empty_values("[a]\nk =\nj=\n", "a", &[("k", ""), ("j", "")])]
#[case::first_delimiter_wins("[a]\nk = a=b\nj: c:d\nm:n=o\n", "a", &[("k", "a=b"), ("j", "c:d"), ("m", "n=o")])]
#[case::separator_controls_are_space("[a]\nk\u{1c}=\u{1f}v\u{1d}\n", "a", &[("k", "v")])]
#[case::no_final_newline("[a]\nk = v", "a", &[("k", "v")])]
#[case::indented_header("  [a]\nk = v\n", "a", &[("k", "v")])]
#[case::indented_first_option("[a]\n  k = v\nj = w\n", "a", &[("k", "v"), ("j", "w")])]
#[case::tab_continuation("[a]\nk = 1\n\t2\n", "a", &[("k", "1\n2")])]
#[case::continuation_after_empty_value("[a]\nk =\n  x\n", "a", &[("k", "\nx")])]
#[case::same_indent_is_not_continuation("[a]\n k = 1\n j = 2\n", "a", &[("k", "1"), ("j", "2")])]
#[case::unicode("[\u{e9}]\nk = \u{fc}\n", "\u{e9}", &[("k", "\u{fc}")])]
fn parses_like_configparser(
    #[case] text: &str,
    #[case] section: &str,
    #[case] fields: &[(&str, &str)],
) {
    assert_eq!(items(text), one(section, fields));
}

#[test]
fn defaults_fill_every_section() {
    let expected: Parsed = vec![
        (
            "a".to_owned(),
            vec![
                ("d".to_owned(), "2".to_owned()),
                ("e".to_owned(), "3".to_owned()),
                ("k".to_owned(), "v".to_owned()),
            ],
        ),
        (
            "b".to_owned(),
            vec![
                ("d".to_owned(), "1".to_owned()),
                ("e".to_owned(), "3".to_owned()),
            ],
        ),
    ];
    assert_eq!(
        items("[DEFAULT]\nd = 1\ne = 3\n[a]\nk = v\nd = 2\n[b]\n"),
        expected
    );
    assert_eq!(
        items("[DEFAULT]\nd=1\n[a]\nk=v\n[DEFAULT]\ne=2\n"),
        one("a", &[("d", "1"), ("e", "2"), ("k", "v")])
    );
}

#[test]
fn sections_keep_file_order() {
    let names: Vec<String> = items("[b]\nk=1\n[a]\nk=2\n")
        .into_iter()
        .map(|(name, _fields)| name)
        .collect();
    assert_eq!(names, ["b", "a"]);
}

#[rstest]
#[case::duplicate_section("[a]\nk=v\n[a]\nj=w\n")]
#[case::duplicate_option("[a]\nk=v\nK=w\n")]
#[case::duplicate_default_option("[DEFAULT]\nd=1\nd=2\n")]
#[case::option_before_header("k=v\n[a]\n")]
#[case::comment_then_option("# c\nk=v\n")]
#[case::empty_brackets("[]\nk=v\n")]
#[case::no_delimiter("[a]\njust text\n")]
#[case::empty_name("[a]\n= v\n")]
fn rejects_what_configparser_rejects(#[case] text: &str) {
    let error = parse(text.as_bytes()).unwrap_err();
    assert!(!matches!(error, ParseError::Utf8(_)), "{error}");
}

#[test]
fn bytes_that_are_not_utf8_are_their_own_error() {
    assert!(matches!(
        parse(b"[a]\nk = \xff\n"),
        Err(ParseError::Utf8(_))
    ));
}
