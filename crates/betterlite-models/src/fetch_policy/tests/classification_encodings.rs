use rstest::rstest;

use super::support;
use crate::fetch_policy::outcome::{AttemptOutcome, FetchedCatalog, classify_fetch_response};
use crate::fetch_policy::revision::git_blob_id;

const BODY: &str = r#"{"model":{}}"#;

#[derive(Clone, Copy, Debug)]
enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16LeBom,
    Utf16BeBom,
    Utf16LeNoBom,
    Utf16BeNoBom,
    Utf32LeBom,
    Utf32BeBom,
    Utf32LeNoBom,
    Utf32BeNoBom,
}

fn unit_bytes(unit: u16, big_endian: bool) -> [u8; 2] {
    if big_endian {
        unit.to_be_bytes()
    } else {
        unit.to_le_bytes()
    }
}

fn code_bytes(value: u32, big_endian: bool) -> [u8; 4] {
    if big_endian {
        value.to_be_bytes()
    } else {
        value.to_le_bytes()
    }
}

fn utf16(text: &str, big_endian: bool) -> Vec<u8> {
    text.encode_utf16()
        .flat_map(|unit| unit_bytes(unit, big_endian))
        .collect()
}

fn utf32(text: &str, big_endian: bool) -> Vec<u8> {
    text.chars()
        .flat_map(|character| code_bytes(u32::from(character), big_endian))
        .collect()
}

fn encoded(encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => BODY.as_bytes().to_vec(),
        Encoding::Utf8Bom => [&[0xEF, 0xBB, 0xBF][..], BODY.as_bytes()].concat(),
        Encoding::Utf16LeBom => [&[0xFF, 0xFE][..], &utf16(BODY, false)].concat(),
        Encoding::Utf16BeBom => [&[0xFE, 0xFF][..], &utf16(BODY, true)].concat(),
        Encoding::Utf16LeNoBom => utf16(BODY, false),
        Encoding::Utf16BeNoBom => utf16(BODY, true),
        Encoding::Utf32LeBom => [&[0xFF, 0xFE, 0x00, 0x00][..], &utf32(BODY, false)].concat(),
        Encoding::Utf32BeBom => [&[0x00, 0x00, 0xFE, 0xFF][..], &utf32(BODY, true)].concat(),
        Encoding::Utf32LeNoBom => utf32(BODY, false),
        Encoding::Utf32BeNoBom => utf32(BODY, true),
    }
}

#[rstest]
#[case::utf8(Encoding::Utf8)]
#[case::utf8_bom(Encoding::Utf8Bom)]
#[case::utf16_le_bom(Encoding::Utf16LeBom)]
#[case::utf16_be_bom(Encoding::Utf16BeBom)]
#[case::utf16_le_no_bom(Encoding::Utf16LeNoBom)]
#[case::utf16_be_no_bom(Encoding::Utf16BeNoBom)]
#[case::utf32_le_bom(Encoding::Utf32LeBom)]
#[case::utf32_be_bom(Encoding::Utf32BeBom)]
#[case::utf32_le_no_bom(Encoding::Utf32LeNoBom)]
#[case::utf32_be_no_bom(Encoding::Utf32BeNoBom)]
fn catalog_fetch_policy_behavior_tests_behavior(#[case] encoding: Encoding) {
    let body = encoded(encoding);
    let response = support::response_bytes(200, body.clone());
    let fetched = reloaded(classify_fetch_response(response, support::URL));
    assert_eq!(fetched.revision, git_blob_id(&body));
    assert_eq!(fetched.body, body);
    assert_eq!(
        fetched
            .models
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["model"]
    );
}

fn reloaded(outcome: AttemptOutcome) -> FetchedCatalog {
    match outcome {
        AttemptOutcome::Reloaded(fetched) => fetched,
        AttemptOutcome::Unavailable(_) | AttemptOutcome::Retryable(_) => {
            panic!("expected reloaded")
        }
    }
}
