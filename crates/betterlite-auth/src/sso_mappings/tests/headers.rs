use indexmap::IndexMap;
use rstest::rstest;

use crate::sso_mappings::headers::{HeaderError, decode_generic_headers};

fn headers(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn an_unset_setting_has_no_headers() {
    assert_eq!(decode_generic_headers(None), Ok(IndexMap::new()));
}

#[rstest]
#[case::empty(b"".as_slice())]
#[case::only_commas(b",, ,".as_slice())]
fn nothing_filled_in_has_no_headers(#[case] raw: &[u8]) {
    assert_eq!(decode_generic_headers(Some(raw)), Ok(IndexMap::new()));
}

#[test]
fn decodes_key_value_pairs() {
    let decoded = decode_generic_headers(Some(b"X-Team=eng,X-Env=prod"));
    assert_eq!(
        decoded,
        Ok(headers(&[("X-Team", "eng"), ("X-Env", "prod")]))
    );
}

#[test]
fn strips_each_segment_but_keeps_its_inner_space() {
    let raw = b"X-Team = eng, X-Team=ops, X-Env=first, X-Env=second, , X-Empty=";
    assert_eq!(
        decode_generic_headers(Some(raw)),
        Ok(headers(&[
            ("X-Team ", " eng"),
            ("X-Team", "ops"),
            ("X-Env", "second"),
            ("X-Empty", ""),
        ]))
    );
}

#[rstest]
#[case::separator_control(b"\x1cX-Team=eng\x1f".as_slice())]
#[case::no_break_space("\u{a0}X-Team=eng\u{a0}".as_bytes())]
#[case::ideographic_space("\u{3000}X-Team=eng\u{3000}".as_bytes())]
fn strips_the_whitespace_python_strips(#[case] raw: &[u8]) {
    assert_eq!(
        decode_generic_headers(Some(raw)),
        Ok(headers(&[("X-Team", "eng")]))
    );
}

#[test]
fn an_empty_key_is_kept() {
    let decoded = decode_generic_headers(Some(b"=value"));
    assert_eq!(decoded, Ok(headers(&[("", "value")])));
}

#[test]
fn a_segment_without_an_equals_sign_is_an_error() {
    let decoded = decode_generic_headers(Some(b"X-Team=eng, X-Env"));
    assert_eq!(
        decoded,
        Err(HeaderError::MissingEquals {
            header: "X-Env".to_owned(),
        })
    );
}

#[test]
fn a_segment_with_two_equals_signs_is_an_error() {
    let decoded = decode_generic_headers(Some(b"X-Team=eng=prod"));
    assert_eq!(
        decoded,
        Err(HeaderError::TooManyEquals {
            header: "X-Team=eng=prod".to_owned(),
        })
    );
}

#[test]
fn bytes_that_are_not_utf8_are_an_error() {
    assert_eq!(
        decode_generic_headers(Some([0xff, b'=', 0xfe].as_slice())),
        Err(HeaderError::NotText)
    );
}
